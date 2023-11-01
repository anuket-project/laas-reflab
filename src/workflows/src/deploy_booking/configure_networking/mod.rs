//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use models::dal::ID;
//use macaddr::MacAddr;
use serde::{Deserialize, Serialize};
use tascii::{prelude::*, task_trait::AsyncRunnable};

use common::prelude::macaddr;

use crate::resource_management::{
    cisco::{self},
    network::NetworkConfig,
};

#[derive(Serialize, Deserialize)]
pub enum MacAddr {
    V6(macaddr::MacAddr6),
    V8(macaddr::MacAddr8),
}

/// A switch identifier holds the FQDN
/// for the switch

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct ConfigureNetworking {
    pub net_config: NetworkConfig,
}

tascii::mark_task!(ConfigureNetworking);
impl AsyncRunnable for ConfigureNetworking {
    type Output = bool;

    async fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        cisco::nx_run_network_task(self.net_config.clone()).await;
        // Current Objective: Figure out what this is supposed to do
        // Next Objective: Make it do what it is supposed to do
        Ok(true)
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("ConfigureNetworkingTask").versioned(1)
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(600.0)
    }

    fn retry_count(&self) -> usize {
        5
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct ApplyNetworkConfig {
    pub net_config: NetworkConfig,
    pub host: ID,
}

tascii::mark_task!(ApplyNetworkConfig);
impl Runnable for ApplyNetworkConfig {
    type Output = bool;

    fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        todo!()
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("ApplyNetworkConfigTask").versioned(1)
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(120.0)
    }

    fn retry_count(&self) -> usize {
        0
    }
}
