use std::time::Duration;

use common::prelude::{tokio, tracing};

use serde::{Deserialize, Serialize};
use tascii::prelude::*;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct WaitSshReachable {
    pub endpoint: String
}

tascii::mark_task!(WaitSshReachable);
/// Returns Ok() if reachable
impl AsyncRunnable for WaitSshReachable {
    type Output = ();

    async fn execute_task(&mut self, _context: &Context, ) -> Result<Self::Output, TaskError> {
        let hostname = self.endpoint.clone();

        loop {
            let res = common::prelude::tokio::process::Command::new("nc")
                .args(["-z", hostname.as_str(), "22"])
                .output()
                .await;

            if let Ok(res) = res {
                if res.status.success() {
                    tracing::info!("Identified ssh server is up on {hostname}");
                    return Ok(());
                }
            } else {
                tracing::info!("Command exited with status code no work: Full: {:?}", res);
            }

            tracing::info!("Endpoint {hostname} wasn't reachable over SSH on port 22 yet, retrying...");
            tokio::time::sleep(Duration::from_secs(3)).await;
        }

    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("WaitSshReachableTask").versioned(1)
    }

    fn timeout() -> Duration {
        Duration::from_secs(10 * 60)
    }

    fn retry_count() -> usize {
        0
    }
}
