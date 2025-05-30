//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::{tokio, tracing};
use dal::{new_client, AsEasyTransaction, FKey, ID};

use models::inventory;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tascii::prelude::*;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct WaitHostOSReachable {
    pub host_id: FKey<inventory::Host>,
    pub timeout: Duration,
}

tascii::mark_task!(WaitHostOSReachable);
impl AsyncRunnable for WaitHostOSReachable {
    type Output = bool;

    fn summarize(&self, id: ID) -> String {
        format!("WaitHostOSReachable with id {id}")
    }

    async fn run(
        &mut self,
        _context: &tascii::prelude::Context,
    ) -> Result<Self::Output, tascii::prelude::TaskError> {
        let end = std::time::Instant::now() + self.timeout;

        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let hostname = self
            .host_id
            .get(&mut transaction)
            .await
            .unwrap()
            .fqdn
            .clone();

        transaction.commit().await.unwrap();

        tracing::info!("Waiting for host \"{hostname}\" to be reachable by ping");

        while Instant::now() < end {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let res = common::prelude::tokio::process::Command::new("ping")
                .args(["-c", "1", hostname.as_str()])
                .output()
                .await;

            if let Ok(res) = res {
                if res.status.success() {
                    return Ok(true);
                }
            }

            tracing::info!("Host {hostname} wasn't reachable yet, retrying...");
        }

        Err(tascii::prelude::TaskError::Reason(
            "waiting for host to be reachable timed out".to_string(),
        ))
    }

    fn variable_timeout(&self) -> Duration {
        // we already time ourselves out by giving up after a while
        self.timeout + Duration::from_secs(20)
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("WaitHostOsReachableTask").versioned(1)
    }
}
