use dal::ID;
use tokio::time;
use std::time::{Duration, Instant};

use common::prelude::{tokio, tracing};

use serde::{Deserialize, Serialize};
use tascii::prelude::*;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct DebugTask {
    pub succeed_when: usize
}

tascii::mark_task!(DebugTask);
impl AsyncRunnable for DebugTask {
    type Output = String;

    async fn execute_task(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        let uid = Uuid::new_v4();

        while true {
            tracing::info!("{uid} is running");
            time::sleep(Duration::from_secs(2)).await
        }

        panic!("I should never get here")
    }

    fn summarize(&self, _id: ID) -> String {
        "Debug Task".to_owned()
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("DebugTask").versioned(1)
    }

    fn timeout() -> Duration {
        Duration::from_secs(15)
    }

    fn retry_count() -> usize {
        3
    }
}
