mod clean_host;

use common::prelude::tracing;
use dal::{new_client, AsEasyTransaction, FKey, ID};
use models::{
    allocator::ResourceHandle,
    dashboard::{Aggregate, LifeCycleState, StatusSentiment},
    EasyLog,
};
use serde::{self, Deserialize, Serialize};
use tascii::prelude::*;

use crate::resource_management::{allocator, vpn::SyncVPN};

use self::clean_host::CleanupHost;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct CleanupAggregate {
    pub agg_id: FKey<Aggregate>,
}

tascii::mark_task!(CleanupAggregate);
impl AsyncRunnable for CleanupAggregate {
    type Output = ();

    fn summarize(&self, id: ID) -> String {
        format!(
            "CleanupAggregate task with id {id}, cleaning up agg {:?}",
            self.agg_id
        )
    }

    async fn run(
        &mut self,
        context: &tascii::prelude::Context,
    ) -> Result<Self::Output, tascii::prelude::TaskError> {
        // this just wants to be best effort, so don't
        // worry too much about retry logic *here*
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let mut agg = self.agg_id.get(&mut transaction).await.unwrap();

        if let LifeCycleState::Active = agg.state {
            tracing::info!("Booking was active, so we won't be conflicting with some other task")
        } else {
            tracing::error!("Booking wasn't active! Tried to deprovision a booking that is either being provisioned still, or is expired. Current state of it was {:?}", agg.state);
            panic!("bad cleanup state");
        }

        let mut cleanup_handles = Vec::new();

        for instance in agg.instances(&mut transaction).await.unwrap().into_iter() {
            let instance = instance.into_inner();

            if let Some(host) = instance.linked_host {
                // verify that the host is still allocated to us,
                // so that it's safe to do this cleanup
                let handle = ResourceHandle::handle_for_host(&mut transaction, host).await?;
                if !handle
                    .currently_owned_by(&mut transaction, self.agg_id)
                    .await?
                {
                    tracing::warn!(
                        "A cleanup task tried to run for agg {:?}, \
                        for host {host:?}, but the host was not allocated \
                        to the aggregate",
                        self.agg_id
                    );

                    continue;
                } else {
                    // we are the owner, so should clean up the host now
                    let jh = context.spawn(CleanupHost {
                        instance: instance.id,
                        agg_id: self.agg_id,
                        host_id: host,
                    });
                    cleanup_handles.push(jh);
                }
            }
        }

        for handle in cleanup_handles {
            let _ignore = handle.join();
        }

        // now, deallocate the aggregate
        allocator::Allocator::instance()
            .deallocate_aggregate(&mut transaction, self.agg_id)
            .await
            .expect("couldn't dealloc agg");

        for instance in agg.instances(&mut transaction).await.unwrap().iter() {
            instance
                .id
                .log(
                    "Cleanup Finished",
                    "host has been deprovisioned and returned to the free pool",
                    StatusSentiment::Succeeded,
                )
                .await;
        }

        agg.state = LifeCycleState::Done;
        agg.update(&mut transaction).await.unwrap();
        transaction.commit().await.unwrap();

        // LifeCycleState is now Done, sync vpn and remove groups from user if needed
        let _ignore = context
            .spawn(SyncVPN {
                users: agg.users.to_owned(),
            })
            .join();

        Ok(())
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("CleanAggTask").versioned(1)
    }
}
