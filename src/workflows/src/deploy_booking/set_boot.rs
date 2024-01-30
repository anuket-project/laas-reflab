//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

#![allow(non_snake_case, non_camel_case_types)]

use common::prelude::{reqwest, tokio, tracing};
use models::inventory::*;
use serde::{Deserialize, Serialize};
use serde_xml_rs::from_str;
use std::{fmt::Display, process::Command, time::Duration, *};
use tascii::{prelude::*, task_trait::AsyncRunnable};

use models::dal::{new_client, AsEasyTransaction, FKey, ID};

use crate::deploy_booking::reachable::WaitReachable;
type BootDevice = (String, String);

pub enum ILOCommand {
    GetPersistentBoot,
    SetPersistentBoot(String),
    SetOneTimeBoot(String),
}

impl Display for ILOCommand {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ILOCommand::GetPersistentBoot => write!(f, "<GET_PERSISTENT_BOOT/>"),
            ILOCommand::SetPersistentBoot(info) => {
                write!(f, "<SET_PERSISTENT_BOOT>{info}</SET_PERSISTENT_BOOT>")
            }
            ILOCommand::SetOneTimeBoot(info) => {
                write!(f, r#"<SET_ONE_TIME_BOOT value = "{info}"/>"#)
            }
        }
    }
}
// Structs for XML Parsing -
//       - Strange capitalization needed for the serde_xml_rs functions
//       - Changes in the XML tag naming scheme will break this process
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct RIBCL {
    PERSISTENT_BOOT: PERSISTENT_BOOT,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct PERSISTENT_BOOT {
    DEVICE: Vec<DEVICE_TAG>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct DEVICE_TAG {
    value: String,
    DESCRIPTION: String,
}

// Main task
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct SetBoot {
    pub host_id: FKey<Host>,
    pub persistent: bool,
    pub boot_to: BootTo,
}

tascii::mark_task!(SetBoot);
impl AsyncRunnable for SetBoot {
    type Output = bool;

    async fn run(
        &mut self,
        context: &tascii::prelude::Context,
    ) -> Result<Self::Output, tascii::prelude::TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host = self
            .host_id
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner();

        transaction.commit().await.unwrap();

        // make sure we can reach the IPMI endpoint
        tracing::info!(
            "Checking that we can reach the IPMI endpoint before we try managing the host"
        );

        let host_url = context
            .spawn(WaitReachable {
                endpoint: host.ipmi_fqdn.clone(),
                timeout: Duration::from_secs(120),
            })
            .join()?;

        let arch = host.arch;

        let result = match arch {
            Arch::Aarch64 => set_arm_boot(&host_url, host, self.persistent, self.boot_to).await,
            Arch::X86 => set_hpe_boot(&host_url, host, self.persistent, self.boot_to).await,
            Arch::X86_64 => set_hpe_boot(&host_url, host, self.persistent, self.boot_to).await,
        };

        return match result {
            true => Ok(true),
            false => Err(TaskError::Reason("Failed to set boot device.".to_owned())),
        };
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("SetBootTask").versioned(1)
    }

    fn summarize(&self, id: ID) -> String {
        format!("[{id} | Set Boot]")
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(600.0)
    }

    fn retry_count(&self) -> usize {
        0
    }
}

// Helper functions

// use xmlrpc script and http request to configure the ilo
// Todo - test and add useful return type
async fn set_hpe_boot(host_url: &str, host: Host, persistent: bool, boot_to: BootTo) -> bool {
    if !persistent {
        let res = run_ilo_command(
            host_url,
            &host,
            ILOCommand::SetOneTimeBoot(
                match boot_to {
                    BootTo::Network => "NETWORK",
                    BootTo::Disk => "HDD",
                }
                .to_owned(),
            ),
        )
        .await;

        return match res {
            Err(_) => false,
            Ok(_) => true,
        };
    }

    // Get all boot devices for a given host, then set the persistent boot order
    let mut boot_device_list: Vec<BootDevice> = xml_to_boot_device_list(
        run_ilo_command(host_url, &host, ILOCommand::GetPersistentBoot)
            .await
            .unwrap(),
    );
    match boot_to {
        BootTo::Network => boot_device_list = network_first_order(boot_device_list),
        BootTo::Disk => boot_device_list = network_last_order(boot_device_list),
    }

    tracing::info!("Sets order for server to {boot_device_list:?}");

    let boot_order: String = boot_device_list_to_string(boot_device_list);

    tracing::info!("Sends command with boot order to host:\n{boot_order}");

    let res = run_ilo_command(host_url, &host, ILOCommand::SetPersistentBoot(boot_order)).await;

    return match res {
        Err(_) => false,
        Ok(_) => true,
    };
}

async fn set_arm_boot(host_url: &str, host: Host, persistent: bool, boot_to: BootTo) -> bool {
    let bdev = match boot_to {
        BootTo::Network => "pxe",
        BootTo::Disk => "disk",
    };

    let mut opts = String::from("options=efiboot");

    if persistent {
        opts.push_str(", persistent");
    }


    tracing::info!("note: going to set bootdev in ipmi multiple times so it really sticks");
    for _i in 0..2 {
        let mut ipmi_cmd = Command::new("ipmitool");

        let mut ipmi_cmd = ipmi_cmd.args([
            "-I",
            "lanplus",
            "-H",
            &host_url,
            "-U",
            &host.ipmi_user,
            "-P",
            &host.ipmi_pass,
            "chassis",
            "bootdev",
            &bdev,
        ]);

        if let BootTo::Network = boot_to {
            ipmi_cmd = ipmi_cmd.arg("set").arg("force_pxe").arg("true");
        } else if let BootTo::Disk = boot_to {
            ipmi_cmd = ipmi_cmd.arg("set").arg("force_disk").arg("true");
        }

        let ipmi_cmd = ipmi_cmd.arg(if persistent {
            "options=efiboot,persistent"
        } else {
            "options=efiboot"
        });
        let ipmi_cmd = ipmi_cmd
            .output()
            .expect("Failed to execute ipmitool command");
        let output2 = str::from_utf8(&ipmi_cmd.stdout).unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
        tracing::info!("IPMI set bootdev returns output: {output2}");
    }

    // todo - extract error code from output. If error, then fail task
    // if output1.error_code != 0 return

    true
}

async fn run_ilo_command(
    host_url: &str,
    host: &Host,
    command: ILOCommand,
) -> Result<String, reqwest::Error> {
    // Runs a command on the ilo using ribcl scripts
    // Returns command output

    //let fqdn = host.ipmi_fqdn.clone();
    let fqdn = host_url;
    let user = host.ipmi_user.clone();
    let password = host.ipmi_pass.clone();

    let mode = match command {
        ILOCommand::SetPersistentBoot(_) => "write",
        ILOCommand::SetOneTimeBoot(_) => "write",
        ILOCommand::GetPersistentBoot => "read",
    };

    let ribcl_script = format!(
        r#"
    <?xml version="1.0"?>
    <?iol entity-procesing="standard"?>
    <?xmlilo output-format="xml"?>
    <RIBCL VERSION="2.0">
      <LOGIN USER_LOGIN="{user}" PASSWORD="{password}">
        <SERVER_INFO MODE="{mode}">
            {command}
        </SERVER_INFO>
      </LOGIN>
    </RIBCL>"#
    );

    let url = String::from(format!("http://{fqdn}/ribcl"));

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let response = client.post(url).body(ribcl_script).send().await;

    match response {
        Ok(_) => response.unwrap().text().await,
        Err(_) => Err(response.err().expect("Expected error.")),
    }
}


fn order(a: &(String, String), b: &(String, String), net_first: bool) -> cmp::Ordering {
    let a_d = a.1.to_ascii_lowercase();
    let b_d = b.1.to_ascii_lowercase();

    let f = |_a: &String, _b: &String| match (a_d.contains("pxe"), b_d.contains("pxe")) {
        (false, false) => cmp::Ordering::Equal,
        (true, false) => cmp::Ordering::Greater,
        (false, true) => cmp::Ordering::Less,
        (true, true) => match (a_d.contains("v4"), b_d.contains("v4")) {
            (false, false) | (true, true) => cmp::Ordering::Equal,
            (true, false) => cmp::Ordering::Greater,
            (false, true) => cmp::Ordering::Less,
        },
    };

    if net_first {
        f(&a_d, &b_d).reverse()
    } else {
        f(&a_d, &b_d)
    }
}

fn network_first_order(mut devices: Vec<BootDevice>) -> Vec<BootDevice> {
    devices.sort_by(|a, b| order(a, b, true));

    // Todo - Add support for pxe first, ipv4 first, etc.
    /*let mut reordered: VecDeque<BootDevice> = VecDeque::new();



    for e in devices.into_iter() {
        let has_net = e.1.to_ascii_lowercase().contains("network");

        if has_net {
            reordered.push_front(e);
        } else {
            reordered.push_back(e);
        }
    }

    let reordered: Vec<BootDevice> = reordered.into_iter().collect();*/

    devices
}

fn network_last_order(mut devices: Vec<BootDevice>) -> Vec<BootDevice> {
    devices.sort_by(|a, b| order(a, b, false));

    devices
}

fn xml_to_boot_device_list(xml: String) -> Vec<BootDevice> {
    let mut devices: Vec<BootDevice> = Vec::new();

    let ribcl: RIBCL;
    let res = from_str(&xml);
    match res {
        Ok(_) => ribcl = res.unwrap(),
        Err(msg) => {
            tracing::error!("Failed to set boot got: {:?}", msg);
            panic!()
        }
    }

    for device in ribcl.PERSISTENT_BOOT.DEVICE {
        devices.push((device.value, device.DESCRIPTION));
    }
    devices
}

fn boot_device_list_to_string(devices: Vec<BootDevice>) -> String {
    let mut ret = String::from("");
    for device in devices {
        let device_value = device.0;
        ret.push_str(&format!(r#"<DEVICE value="{device_value}"/>"#));
    }

    ret
}
