use serde::{Deserialize, Serialize};
use tascii::{prelude::*, task_trait::AsyncRunnable};

use common::prelude::macaddr;

use super::types::NetworkConfig;
use crate::resource_management::cisco::{self};

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

    async fn execute_task(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        cisco::nx_run_network_task(self.net_config.clone()).await;
        // sonic::sonic_run_network_task(self.net_config.clone()).await;
        Ok(true)
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("ConfigureNetworkingTask").versioned(1)
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(600.0)
    }

    fn retry_count() -> usize {
        2
    }
}
