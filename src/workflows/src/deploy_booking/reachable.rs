//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use common::prelude::{tokio, tracing};

use serde::{Deserialize, Serialize};
use tascii::prelude::*;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct WaitReachable {
    pub endpoint: String,
    pub timeout: Duration,
}

tascii::mark_task!(WaitReachable);
/// Returns Ok() if reachable, with the IP
/// that responded to the ping, or if that
/// can't be determined the original endpoint
impl AsyncRunnable for WaitReachable {
    type Output = String;

    async fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        let hostname = self.endpoint.clone();

        let end = std::time::Instant::now() + self.timeout;
        while Instant::now() < end {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let res = common::prelude::tokio::process::Command::new("ping")
                .args(["-c", "1", "-n", "-q", &hostname.as_str()])
                .output()
                .await;

            if let Ok(res) = res {
                let stdout = String::from_utf8(res.stdout).unwrap_or(String::new());
                let responder: Option<String> = (|| {
                    let first = stdout.lines().next()?;
                    let (_, rest) = first.split_once("(")?;
                    let (ip, _) = rest.split_once(")")?;

                    tracing::info!("Identified that host is reachable at {ip}");

                    Some(ip.to_owned())
                })();

                if res.status.success() {
                    return Ok(responder.unwrap_or(self.endpoint.clone()));
                }
            }

            tracing::info!("Endpoint {hostname} wasn't reachable yet, retrying...");
        }

        Err(TaskError::Timeout())
    }

    fn summarize(&self, _id: models::dal::ID) -> String {
        todo!()
    }

    fn variable_timeout(&self) -> Duration {
        // we already time ourselves out by giving up after a while
        self.timeout + Duration::from_secs(20)
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("WaitReachableTask").versioned(1)
    }
}
