//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap},
    sync::Arc,
};

use common::prelude::{dashmap::DashMap, lazy_static, parking_lot, tracing};
use lazy_static::lazy_static;


use super::network::NetworkConfig;
use models::dal::{new_client, AsEasyTransaction};

#[derive(Clone)]
pub struct NXCommand {
    inputs: Vec<String>,

    url: String,

    user: String,
    password: String,
}

impl std::fmt::Debug for NXCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NXCommand")
            .field("inputs", &self.inputs)
            .field("url", &self.url)
            .field("user", &"<redacted>")
            .field("password", &"<redacted>")
            .finish()
    }
}

pub struct NXCommandWithoutAuth {
    url: String,
}

impl NXCommandWithoutAuth {
    pub fn with_credentials(self, username: String, password: String) -> NXCommand {
        NXCommand {
            inputs: vec![],
            url: self.url,
            user: username,
            password,
        }
    }
}

impl NXCommand {
    pub fn for_switch(dn: String) -> NXCommandWithoutAuth {
        NXCommandWithoutAuth {
            url: format!("http://{dn}/ins"),
        }
    }

    pub fn and_then<S>(mut self, command: S) -> Self
    where S: Into<String> {
        self.inputs.push(command.into());
        self
    }

    pub fn execute(self) {
        tracing::info!("Getting switch lock");
        let lock = SWITCH_LOCK
            .entry(self.url.clone())
            .or_insert_with(|| Arc::new(parking_lot::Mutex::new(())))
            .value()
            .clone();
        tracing::info!("Got lock, now waiting for exclusive access to {}", self.url);

        let g = lock.lock();

        let concat_input = self
            .inputs
            .into_iter()
            .intersperse(" ; ".to_string())
            .reduce(|acc, e| acc + e.as_str());

        let j = ureq::json!({
            "ins_api": {
                "version": "1.0",
                "type": "cli_conf",
                "chunk": "0",
                "sid": "1",
                "output_format": "json",
                "input": concat_input,
            }
        });

        let basic_auth_header_v = format!("{}:{}", self.user, self.password);

        // TODO
        #[allow(deprecated)]
        let basic_auth_header = format!("Basic {}", base64::encode(basic_auth_header_v.as_bytes()));

        let resp = ureq::post(&self.url)
            .set("Authorization", &basic_auth_header)
            .set("content-type", "text/json")
            .send_json(j)
            .expect("couldn't send request to switch");

        tracing::info!("Releases exclusive access to switch {}", self.url);
        std::mem::drop(g);

        tracing::warn!(
            "got back resp from switch, status: {}, text: {:#?}",
            resp.status(),
            resp.into_string()
        );
    }
}

lazy_static! {
    static ref SWITCH_LOCK: DashMap<String, Arc<common::prelude::parking_lot::Mutex<()>>> =
        DashMap::new();
}

pub async fn nx_run_network_task(nc: NetworkConfig) {
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await.unwrap();

    let mut for_switch = None;

    let mut switches = HashMap::new();
    //let command = NXCommand::for_switch(dn)
    for bondgroup in nc.bondgroups.clone() {
        for member in bondgroup.member_host_ports.iter() {
            let hp = member.get(&mut transaction).await.unwrap();
            if let Some(sp) = hp.switchport {
                let sp = sp.get(&mut transaction).await.unwrap();
                // As all hostports must be connected to the same cisco switch we can safely return without doing anything if any hostport is connected to an edgecore
                if sp.for_switch.get(&mut transaction).await.unwrap().switch_os.unwrap().get(&mut transaction).await.expect("Expected to get OS").os_type != *"NXOS" {
                    return;
                }

                for_switch = match for_switch {
                    None => Some(sp.for_switch),
                    Some(prior) => {
                        assert_eq!(prior, sp.for_switch);

                        Some(prior)
                    }
                }
            }
        }

        let for_switch = for_switch
            .unwrap()
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner();

        
            let nxcommand = switches.entry(for_switch.id).or_insert_with(|| {
                NXCommand::for_switch(for_switch.ip).with_credentials(for_switch.user, for_switch.pass)
            });
    
            tracing::warn!("not supporting/doing actual bond groups yet, just assume each port is in a separate one");
            for port in bondgroup.member_host_ports {
                *nxcommand = nxcommand.clone().and_then(format!(
                    "interface {}",
                    port.get(&mut transaction)
                        .await
                        .unwrap()
                        .switchport
                        .unwrap()
                        .get(&mut transaction)
                        .await
                        .unwrap()
                        .name
                ));
                *nxcommand = nxcommand.clone().and_then("switchport mode trunk");
    
                let mut native_vlan = None;
    
                let mut allowed_vlans = Vec::new();
    
                for vlan_connection in bondgroup.vlans.clone() {
                    let vlan = vlan_connection.vlan.get(&mut transaction).await.unwrap();
                    if !vlan_connection.tagged {
                        assert!(
                            native_vlan.replace(vlan.vlan_id).is_none(),
                            "already had a native vlan?"
                        );
                    }
    
                    allowed_vlans.push(vlan.vlan_id);
                }
    
                allowed_vlans.sort(); // try to make them always incrementing to avoid complaints from
                                      // switches
    
                let allowed_vlans_string = allowed_vlans
                    .into_iter()
                    .map(|vlid| vlid.to_string())
                    .intersperse(",".to_string())
                    .reduce(|acc, e| acc + e.as_str());
    
                if let Some(vlans) = allowed_vlans_string {
                    *nxcommand = nxcommand
                        .clone()
                        .and_then(format!("switchport trunk allowed vlan {vlans}"));
                }
    
                if let Some(nvlid) = native_vlan {
                    *nxcommand = nxcommand
                        .clone()
                        .and_then(format!("switchport trunk native vlan {nvlid}"));
                } else {
                    *nxcommand = nxcommand
                        .clone()
                        .and_then("no switchport trunk native vlan");
                }
    
                tracing::info!("running command on switch: {nxcommand:#?}");
            }
    }

    for (_sw, nxc) in switches {
        // now we coalesce them so they're a bit less dumb
        let nxc = if nc.persist {
            nxc.and_then("copy run start")
        } else {
            nxc
        };
        nxc.execute();
    }

    transaction.commit().await.unwrap();
}
