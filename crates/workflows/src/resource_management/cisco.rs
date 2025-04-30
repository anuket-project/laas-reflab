use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, Result};
use common::prelude::{dashmap::DashMap, lazy_static, parking_lot, tracing};
use lazy_static::lazy_static;
use models::inventory::{HostPort, Switch};

use crate::configure_networking::{BondGroup, NetworkConfig, VlanConnection};
use dal::{new_client, AsEasyTransaction, EasyTransaction, FKey};

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
    where
        S: Into<String>,
    {
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

        tracing::info!(
            "Here is every single nx command being run on the switch: {:#?}",
            self.inputs
        );

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

/// Represents the VLAN state of a switch port.
/// This type is serialized into NX-OS commands to run against a switch to configure a switch port.
#[derive(Debug)]
pub enum SwitchPortVlanState {
    Disabled,
    Tagged(Vec<i16>),
    Native(i16),
    TaggedAndNative {
        allowed_vlans: Vec<i16>,
        native_vlan: i16,
    },
}

impl SwitchPortVlanState {
    /// Converts the VLAN state into `Vec<String>` that represent NX-OS commands.
    pub fn to_nx_commands(&self) -> Vec<String> {
        match self {
            // Disabled state
            SwitchPortVlanState::Disabled => vec!["shutdown".to_string()],
            // Only Tagged
            SwitchPortVlanState::Tagged(allowed_vlans) => vec![
                "switchport mode trunk".to_string(),
                // disable native vlan
                "no switchport trunk native vlan".to_string(),
                format!(
                    "switchport trunk allowed vlan {}",
                    allowed_vlans_to_string(allowed_vlans)
                ),
                "no shutdown".to_string(),
            ],
            // Only Native
            SwitchPortVlanState::Native(native_vlan) => vec![
                "switchport mode trunk".to_string(),
                format!("switchport trunk native vlan {}", native_vlan),
                // If Native VLAN is set, it must also be explicitly allowed
                format!("switchport trunk allowed vlan {}", native_vlan),
                "no shutdown".to_string(),
            ],
            SwitchPortVlanState::TaggedAndNative {
                allowed_vlans,
                native_vlan,
            } => {
                let mut all_vlans = allowed_vlans.clone();
                all_vlans.push(*native_vlan);
                vec![
                    "switchport mode trunk".to_string(),
                    // If Native VLAN is set, it must also be explicitly allowed
                    format!(
                        "switchport trunk allowed vlan {}",
                        allowed_vlans_to_string(&all_vlans)
                    ),
                    format!("switchport trunk native vlan {}", native_vlan),
                    "no shutdown".to_string(),
                ]
            }
        }
    }
}

pub async fn nx_run_network_task(network_config: NetworkConfig) {
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await.unwrap();

    // Process each bond group
    let switches = process_bondgroups(&network_config, &mut transaction).await;

    // Apply commands to switches
    apply_commands_to_switches(switches, network_config.persist).await;

    transaction.commit().await.unwrap();
}

async fn process_bondgroups(
    network_config: &NetworkConfig,
    transaction: &mut EasyTransaction<'_>,
) -> HashMap<FKey<Switch>, NXCommand> {
    let mut switches = HashMap::new();
    for bondgroup in network_config.bondgroups.clone() {
        process_bondgroup(&bondgroup, transaction, &mut switches).await;
    }
    switches
}

async fn process_bondgroup(
    bondgroup: &BondGroup,
    transaction: &mut EasyTransaction<'_>,
    switches: &mut HashMap<FKey<Switch>, NXCommand>,
) {
    let mut for_switch = None;

    for member in bondgroup.member_host_ports.iter() {
        let switch = validate_hostport(member, transaction)
            .await
            .expect("Failed to validate host port");

        // update `for_switch`
        for_switch = match for_switch {
            None => Some(switch),
            Some(prior) => {
                // ensure that all switches are the same
                if prior != switch {
                    panic!("All switches must be the same for a bond group");
                }
                Some(prior)
            }
        };
    }

    if let Some(for_switch) = for_switch {
        let for_switch = for_switch.get(transaction).await.unwrap().into_inner();

        let nxcommand = switches.entry(for_switch.id).or_insert_with(|| {
            NXCommand::for_switch(for_switch.ip).with_credentials(for_switch.user, for_switch.pass)
        });

        process_ports(bondgroup, transaction, nxcommand).await;
    }
}

async fn validate_hostport(
    member: &FKey<HostPort>,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Switch>> {
    // fetch the `HostPort`
    let host_port = member
        .get(transaction)
        .await
        .map_err(|e| anyhow!("Failed to get HostPort: {}", e))?;

    // fetch the associated `SwitchPort` from the host port
    let switch_port = host_port
        .switchport
        .get(transaction)
        .await
        .map_err(|e| anyhow!("Failed to get SwitchPort: {}", e))?;

    // fetch the associated `Switch` from the switch port
    let switch = switch_port
        .for_switch
        .get(transaction)
        .await
        .map_err(|e| anyhow!("Failed to get Switch: {}", e))?;

    // fetch the associated `SwitchOS` from the swictch
    let switch_os = switch
        .switch_os
        .ok_or_else(|| anyhow!("SwitchOS is not set for the Switch"))?
        .get(transaction)
        .await
        .map_err(|e| anyhow!("Failed to get SwitchOS: {}", e))?;

    // check if the OS type is "NXOS"
    if switch_os.os_type != "NXOS" {
        return Err(anyhow!("Switch OS type is not NXOS"));
    }

    Ok(switch_port.for_switch)
}

async fn process_ports(
    bondgroup: &BondGroup,
    transaction: &mut EasyTransaction<'_>,
    nxcommand: &mut NXCommand,
) {
    tracing::warn!(
        "not supporting/doing actual bond groups yet, just assume each port is in a separate one"
    );

    for port in bondgroup.member_host_ports.iter() {
        configure_port(port, transaction, nxcommand, &bondgroup.vlans).await;
    }
}

async fn configure_port(
    port: &FKey<HostPort>,
    transaction: &mut EasyTransaction<'_>,
    nxcommand: &mut NXCommand,
    vlans: &[VlanConnection],
) {
    let switchport = port
        .get(transaction)
        .await
        .unwrap()
        .switchport
        .get(transaction)
        .await
        .unwrap();

    *nxcommand = nxcommand
        .clone()
        .and_then(format!("interface {}", switchport.name));

    let vlan_state = collect_vlan_info(vlans, transaction).await;

    let commands = vlan_state.to_nx_commands();
    for command in commands {
        *nxcommand = nxcommand.clone().and_then(command);
    }

    tracing::info!(
        "Configured switchport {}: {:#?}",
        switchport.name,
        vlan_state
    );
}

async fn collect_vlan_info(
    vlans: &[VlanConnection],
    transaction: &mut EasyTransaction<'_>,
) -> SwitchPortVlanState {
    let mut native_vlan = None;
    let mut allowed_vlans = Vec::new();

    for vlan_connection in vlans {
        let vlan = vlan_connection.vlan.get(transaction).await.unwrap();

        if !vlan_connection.tagged {
            if native_vlan.replace(vlan.vlan_id).is_some() {
                panic!("Multiple untagged VLANs found; only one native VLAN is allowed.");
            }
        } else {
            allowed_vlans.push(vlan.vlan_id);
        }
    }

    allowed_vlans.sort_unstable();

    match (native_vlan, allowed_vlans.is_empty()) {
        (None, true) => SwitchPortVlanState::Disabled,
        (None, false) => SwitchPortVlanState::Tagged(allowed_vlans),
        (Some(native), true) => SwitchPortVlanState::Native(native),
        (Some(native), false) => SwitchPortVlanState::TaggedAndNative {
            allowed_vlans,
            native_vlan: native,
        },
    }
}

async fn apply_commands_to_switches(switches: HashMap<FKey<Switch>, NXCommand>, persist: bool) {
    for (_switch, mut nxcommand) in switches {
        if persist {
            // save the running config to the startup config on the switch
            nxcommand = nxcommand.and_then("copy run start");
        }
        nxcommand.execute();
    }
}

/// Helper function to format VLAN IDs as a comma-separated string.
fn allowed_vlans_to_string(allowed_vlans: &[i16]) -> String {
    allowed_vlans
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}
