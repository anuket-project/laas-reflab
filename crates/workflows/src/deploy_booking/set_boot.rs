//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

#![allow(non_snake_case, non_camel_case_types)]

use common::prelude::{anyhow, reqwest, tokio, tracing};
use models::inventory::*;
use serde::{Deserialize, Serialize};
use serde_xml_rs::from_str;
use std::{cmp::Ordering, fmt::Display, process::Command, time::Duration, *};
use tascii::{prelude::*, task_trait::AsyncRunnable};

use dal::{new_client, AsEasyTransaction, FKey, ID};

use crate::deploy_booking::reachable::WaitReachable;
type BootDevice = (String, String);

#[derive(Debug)]
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
struct Ribcl {
    PERSISTENT_BOOT: PersistentBoot,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct PersistentBoot {
    DEVICE: Vec<DeviceTag>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct DeviceTag {
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
    type Output = ();

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

        let arch = host.flavor.get(&mut transaction).await.unwrap().arch;

        transaction.commit().await.unwrap();

        let result = match arch {
            Arch::Aarch64 => set_ipmi_boot(&host_url, host, self.persistent, self.boot_to).await,
            Arch::X86 => set_hpe_boot(&host_url, host, self.persistent, self.boot_to).await,
            Arch::X86_64 => set_hpe_boot(&host_url, host, self.persistent, self.boot_to).await,
        };

        match result {
            Ok(_) => Ok(()),
            Err(_) => Err(TaskError::Reason("Failed to set boot device.".to_owned())),
        }
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

async fn ilo_persistent_boot(
    host_url: &str,
    host: Host,
    boot_to: BootTo,
) -> Result<(), anyhow::Error> {
    tracing::info!("Attempting to set persistent boot through the ILO.");
    // Get all boot devices for a given host, then set the persistent boot order
    let mut boot_device_list: Vec<BootDevice> = match xml_to_boot_device_list(
        run_ilo_command(host_url, &host, ILOCommand::GetPersistentBoot)
            .await
            .unwrap(),
    ) {
        Ok(l) => l,
        Err(_) => {
            return Err(anyhow::Error::msg(format!(
                "Failed to get boot device list for host {}",
                host.server_name
            )))
        }
    };

    tracing::warn!("Boot device list is {boot_device_list:?}");
    match boot_to {
        BootTo::Network => boot_device_list = network_first_order(boot_device_list),
        BootTo::Disk => {
            boot_device_list = match network_last_order(boot_device_list, None) {
                Ok(l) => l,
                Err(e) => return Err(e),
            }
        }
        BootTo::SpecificDisk => {
            boot_device_list = match network_last_order(boot_device_list, Some(host.clone())) {
                Ok(l) => l,
                Err(e) => return Err(e),
            }
        }
    }

    tracing::info!("Sets order for server to {boot_device_list:?}");

    let boot_order: String = boot_device_list_to_string(boot_device_list);

    tracing::info!("Sends command with boot order to host:\n{boot_order}");

    let res =
        match run_ilo_command(host_url, &host, ILOCommand::SetPersistentBoot(boot_order)).await {
            Ok(s) => s,
            Err(_) => {
                return Err(anyhow::Error::msg(
                    "Failed to run ilo command, URL '{host_url}' may be incorrect",
                ))
            }
        };

    tracing::info!("Result of set persistent boot is {res}");

    Ok(())
}

async fn ilo_one_time_boot(
    host_url: &str,
    host: Host,
    boot_to: BootTo,
) -> Result<(), anyhow::Error> {
    tracing::info!("Attempting to set one time boot through the ILO.");

    let res = match run_ilo_command(
        host_url,
        &host,
        ILOCommand::SetOneTimeBoot(
            match boot_to {
                BootTo::Network => "NETWORK",
                BootTo::Disk | BootTo::SpecificDisk => "HDD",
            }
            .to_owned(),
        ),
    )
    .await
    {
        Ok(s) => s,
        Err(_) => {
            return Err(anyhow::Error::msg(format!(
                "Failed to set one time boot to {boot_to} for {} at URL {host_url}",
                host.server_name
            )))
        }
    };

    tracing::info!("Result of set one time boot is {res:?}");

    Ok(())
}

/**
 * Try to set the boot using xmlrpc and RIBCL
 * If this fails, the host probably isn't an HPE host so just switch over to using ipmitool commands instead
 */
async fn set_hpe_boot(
    host_url: &str,
    host: Host,
    persistent: bool,
    boot_to: BootTo,
) -> Result<(), anyhow::Error> {
    let result = if persistent {
        ilo_persistent_boot(host_url, host.clone(), boot_to).await
    } else {
        ilo_one_time_boot(host_url, host.clone(), boot_to).await
    };

    match result {
        Ok(_) => Ok(()),
        Err(_) => {
            tracing::warn!("Failed to set hpe boot for {host:?}. Trying ipmi instead.");
            set_ipmi_boot(host_url, host, persistent, boot_to).await
        }
    }
}

async fn set_ipmi_boot(
    host_url: &str,
    host: Host,
    persistent: bool,
    boot_to: BootTo,
) -> Result<(), anyhow::Error> {
    tracing::info!(
        "Setting IPMI boot for {host:?} with persistent {persistent} and boot_to {boot_to}"
    );
    let bdev = match boot_to {
        BootTo::Network => "pxe",
        BootTo::Disk | BootTo::SpecificDisk => "disk",
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
            "-C",
            "3",
            "-H",
            host_url,
            "-U",
            &host.ipmi_user,
            "-P",
            &host.ipmi_pass,
            "chassis",
            "bootdev",
            bdev,
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

    Ok(())
}

async fn run_ilo_command(host_url: &str, host: &Host, command: ILOCommand) -> Result<String, ()> {
    tracing::info!("Attempting to run ILO command {command:?}");
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

    let url = format!("http://{fqdn}/ribcl");

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let response = client.post(url).body(ribcl_script).send().await;
    tracing::info!("RIBCL response is {response:?}");

    match response {
        Ok(r) => Ok(r.text().await.unwrap()),
        Err(_) => Err(()),
    }
}

fn order(a: &(String, String), b: &(String, String), net_first: bool) -> cmp::Ordering {
    let a_d = a.1.to_ascii_lowercase();
    let b_d = b.1.to_ascii_lowercase();

    let f = |_a: &String, _b: &String| match (a_d.contains("pxe"), b_d.contains("pxe")) {
        (false, false) => match (a_d.contains("network"), b_d.contains("network")) {
            (false, false) => cmp::Ordering::Equal,
            (true, false) => cmp::Ordering::Greater,
            (false, true) => cmp::Ordering::Less,
            (true, true) => match (a_d.contains("v4"), b_d.contains("v4")) {
                (false, false) | (true, true) => cmp::Ordering::Equal,
                (true, false) => cmp::Ordering::Greater,
                (false, true) => cmp::Ordering::Less,
            },
        },
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

fn network_last_order(
    mut devices: Vec<BootDevice>,
    host: Option<Host>,
) -> Result<Vec<BootDevice>, anyhow::Error> {
    // This should be refactored or documented better. Nothing implies that providing Some(host) means specific disk boot.
    match host {
        Some(h) => match h.sda_uefi_device {
            Some(d) => devices.sort_by(|a, b| sort_device_to_top(a, b, d.clone())),
            None => {
                return Err(anyhow::Error::msg(
                    "Attempting to boot a specific drive, but none are specified for this host",
                ))
            }
        },
        None => devices.sort_by(|a, b| order(a, b, false)),
    }

    Ok(devices)
}

fn xml_to_boot_device_list(xml: String) -> Result<Vec<BootDevice>, ()> {
    let mut devices: Vec<BootDevice> = Vec::new();
    tracing::info!("xml to boot device list: xml is {xml:?}");

    let res = from_str(&xml);
    tracing::info!("res is {res:?}");
    let ribcl: Ribcl = match res {
        Ok(r) => r,
        Err(msg) => {
            tracing::warn!("Failed to get boot device list. Is host an HPE? {:?}", msg);
            return Err(());
        }
    };

    for device in ribcl.PERSISTENT_BOOT.DEVICE {
        devices.push((device.value, device.DESCRIPTION));
    }

    Ok(devices)
}

fn boot_device_list_to_string(devices: Vec<BootDevice>) -> String {
    let mut ret = String::from("");
    for device in devices {
        let device_value = device.0;
        ret.push_str(&format!(r#"<DEVICE value="{device_value}"/>"#));
    }

    ret
}

fn sort_device_to_top(a: &(String, String), b: &(String, String), device: String) -> cmp::Ordering {
    let a_d = a.1.to_ascii_lowercase();
    let b_d = b.1.to_ascii_lowercase();

    let f = |_a: &String, _b: &String| match (
        a_d.contains(device.as_str()),
        b_d.contains(device.as_str()),
    ) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => Ordering::Equal,
    };

    f(&a_d, &b_d)
}
