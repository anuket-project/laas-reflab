use std::time::Duration;

use common::prelude::{tokio, tracing};

use serde::{Deserialize, Serialize};
use tascii::prelude::*;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct WaitReachable {
    pub endpoint: String,
}

tascii::mark_task!(WaitReachable);
/// Returns Ok() if reachable, with the IP
/// that responded to the ping, or if that
/// can't be determined the original endpoint
impl AsyncRunnable for WaitReachable {
    type Output = String;

    async fn execute_task(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        let hostname = self.endpoint.clone();

        while true {
            let res = common::prelude::tokio::process::Command::new("ping")
                .args(["-c", "1", "-n", "-q", hostname.as_str()])
                .output()
                .await;

            if let Ok(res) = res {
                let stdout = String::from_utf8(res.stdout).unwrap_or_default();
                let responder: Option<String> = (|| {
                    let first = stdout.lines().next()?;
                    let (_, rest) = first.split_once('(')?;
                    let (ip, _) = rest.split_once(')')?;

                    tracing::info!("Identified that host is reachable at {ip}");

                    Some(ip.to_owned())
                })();

                if res.status.success() {
                    return Ok(responder.unwrap_or(self.endpoint.clone()));
                }
            }
            tracing::info!("Endpoint {hostname} wasn't reachable yet, retrying...");
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        unreachable!()
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("WaitReachableTask").versioned(1)
    }

    fn timeout() -> Duration {
        Duration::from_secs(10 * 60)
    }

    fn retry_count() -> usize {
        0
    }
}
