use models::{dal::FKey, dashboard::Aggregate};
use serde::{Deserialize, Serialize};

use tascii::{prelude::*, task_trait::AsyncRunnable};

use crate::{deploy_booking::notify::Notify, resource_management::vpn::SyncVPN};

use config::Situation;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct AddUsers {
    pub agg_id: FKey<Aggregate>,
    pub users: Vec<String>,
}

tascii::mark_task!(AddUsers);
impl AsyncRunnable for AddUsers {
    type Output = ();

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("AddUsers").versioned(1)
    }

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        context.spawn(SyncVPN {
            users: self.users.clone(),
        });

        context.spawn(Notify {
            aggregate: self.agg_id,
            situation: Situation::CollaboratorAdded(self.users.clone()),
            extra_context: vec![]
        });

        Ok(())
    }
}
