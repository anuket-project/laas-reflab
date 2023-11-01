//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::{strum_macros::Display, tracing};
use models::{
    dal::{new_client, AsEasyTransaction, FKey, ID},
    inventory::Host,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tascii::{prelude::*, task_trait::AsyncRunnable};

use std::{process::Command, str};

use crate::deploy_booking::reachable::WaitReachable;

#[derive(Serialize, Deserialize, Debug, Hash, Clone, Eq, PartialEq)]
pub struct SetPower {
    pub host: FKey<Host>,
    pub pstate: PowerState,
}

#[derive(Serialize, Deserialize, Debug, Hash, Clone, Eq, PartialEq, Display)]
pub enum PowerState {
    On,
    Off,
    Reset,
    Unkown,
}

impl SetPower {
    pub fn off(host: FKey<Host>) -> Self {
        tracing::warn!("In setpower::off.");
        Self {
            host,
            pstate: PowerState::Off,
        }
    }

    pub fn on(host: FKey<Host>) -> Self {
        tracing::warn!("In setpower::on.");
        Self {
            host,
            pstate: PowerState::On,
        }
    }

    pub fn reset(host: FKey<Host>) -> Self {
        tracing::warn!("In setpower::on.");
        Self {
            host,
            pstate: PowerState::Reset,
        }
    }
}

tascii::mark_task!(SetPower);
impl AsyncRunnable for SetPower {
    type Output = ();

    //Return true if succeeded, else false

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        // std::thread::sleep(Duration::from_secs_f64(5.0)); // TODO: get rid of wait
        tracing::warn!("Setting power.");
        let host = self.host.get(&mut transaction).await.unwrap();
        transaction.commit().await.unwrap();

        let ipmi_fqdn = &host.ipmi_fqdn;
        let ipmi_admin_user = &host.ipmi_user;
        let ipmi_admin_password = &host.ipmi_pass;

        // make sure we can reach the IPMI endpoint
        tracing::info!("Checking that we can reach the IPMI endpoint");
        let ipmi_url = context
            .spawn(WaitReachable {
                endpoint: ipmi_fqdn.clone(),
                timeout: Duration::from_secs(120),
            })
            .join()?;

        tracing::info!(
            "about to run ipmi power on {:?} to set power to {:?}",
            ipmi_fqdn,
            self.pstate
        );

        let ipmitool = Command::new("ipmitool")
            .args([
                "-I",
                "lanplus",
                "-H",
                &ipmi_url,
                "-U",
                &ipmi_admin_user,
                "-P",
                &ipmi_admin_password,
                "chassis",
                "power",
                match self.pstate {
                    PowerState::Off => "off",
                    PowerState::On => "on",
                    PowerState::Reset => "reset",
                    PowerState::Unkown => panic!("bad instruction"),
                },
            ])
            .output()
            .expect("Failed to execute ipmitool command");
        let stdout = String::from_utf8(ipmitool.stdout).expect("no stdout?");
        let stderr = String::from_utf8(ipmitool.stderr).expect("no stderr?");

        tracing::info!("ran ipmitool, output was: {stdout}, with stderr: {stderr}");

        if stderr.contains("Unable to establish IPMI") {
            return Err(TaskError::Reason(format!(
                "IPMItool could not reach the host, host was: {}, fqdn was: {ipmi_fqdn}",
                host.server_name
            )));
        }

        for _ in 0..50 {
            tracing::info!("about to check host power state");
            std::thread::sleep(Duration::from_secs_f64(5.0));
            tracing::info!("checking host power state");
            let current_state =
                get_host_power_state(&ipmi_fqdn, &ipmi_admin_user, &ipmi_admin_password);
            if current_state.eq(&self.pstate) || self.pstate.eq(&PowerState::Reset) {
                tracing::info!("Host reached desired state! :)");
                return Ok(());
            } else {
                match current_state {
                    PowerState::On => {
                        tracing::warn!("Host is not in desired state, is on instead. :(");
                    }
                    PowerState::Off => {
                        tracing::warn!("Host is not in desired state, is off instead. :(");
                    }
                    PowerState::Reset => {
                        continue;
                    }
                    PowerState::Unkown => {
                        continue;
                    }
                }
            }
        }

        return Err(TaskError::Reason(format!("host {} with ipmi fqdn {ipmi_fqdn} failed to reach desired power state, even though it accepted the power command", host.server_name)));
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("SetPowerState").versioned(1)
    }

    fn summarize(&self, id: ID) -> String {
        format!("[{id} | Set Power]")
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(240.0)
    }
}

// Helper Functions - should not be public or accessed outside of these tasks
pub fn get_host_power_state(fqdn: &String, user: &String, password: &String) -> PowerState {
    let ipmitool = Command::new("ipmitool")
        .args([
            "-I", "lanplus", "-H", &fqdn, "-U", &user, "-P", &password, "chassis", "power",
            "status",
        ])
        .output()
        .expect("Failed to execute ipmitool command");

    let output = str::from_utf8(&ipmitool.stdout).unwrap();

    tracing::info!("Chassis power was '{output}'");

    if output.eq("Chassis Power is on\n") {
        return PowerState::On;
    }

    if output.eq("Chassis Power is off\n") {
        return PowerState::Off;
    }

    tracing::warn!("Host is in a _weird_ state: {output}");

    PowerState::Unkown
}
