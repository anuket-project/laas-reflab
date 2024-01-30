//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::*;
use models::{
    dal::{new_client, AsEasyTransaction, FKey, ID},
    dashboard::Aggregate,
};
use serde::{self, Deserialize, Serialize};
use tascii::{
    task_trait::{AsyncRunnable, TaskIdentifier},
};
use users::*;


use super::allocator;

tascii::mark_task!(SyncVPN);
#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub struct SyncVPN {
    pub aggregate: FKey<Aggregate>,
}

impl AsyncRunnable for SyncVPN {
    type Output = ();

    async fn run(
        &mut self,
        _context: &tascii::prelude::Context,
    ) -> Result<Self::Output, tascii::prelude::TaskError> {
        // for each user in aggregate.users
        //   use allocation2::Allocator::instance().active_vpn_for(user) to get the
        //   groups that the user should at least be in
        // iterate through all of those groups and, using the IPA module,
        // if the user isn't in one of the groups, add them
        // if the user is in a group that isn't in that vec/set,
        // remove them from the group
        //

        let mut client = new_client().await.expect("Expected to connect to db");
        let mut transaction = client
            .easy_transaction()
            .await
            .expect("Transaction creation error");

        let mut ipa = ipa::IPA::init().await.expect("Expected to connect to IPA");

        let _all_projects: Vec<String>;
        for user in self
            .aggregate
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner()
            .users
        {
            // users in the agg
            let active_projects = allocator::Allocator::instance()
                .active_vpn_for(&mut transaction, user.clone())
                .await
                .expect("couldn't query ipa, bad user?");

            let all_projects: Vec<String> = config::settings()
                .projects
                .keys()
                .map(|p| p.to_owned())
                .collect();

            // check what groups the user is in
            for group in all_projects {
                let in_group = active_projects
                    .iter()
                    .find(|project| group.eq_ignore_ascii_case(&project))
                    .is_some();

                match in_group {
                    true => {
                        match ipa
                            .group_add_user(group.clone(), user.clone())
                            .await
                            .unwrap()
                        {
                            true => {
                                tracing::info!("Added user to group");
                                tracing::warn!("Notify user they were added to ipa group");
                            }
                            false => {
                                tracing::info!(
                                    "Failed to add user to group, may already be in group?"
                                );
                            }
                        }
                    }
                    false => {
                        match ipa
                            .group_remove_user(group.clone(), user.clone())
                            .await
                            .unwrap()
                        {
                            true => {
                                tracing::info!("Removed user {user} from group {group}");
                                tracing::warn!("Notify user they were removed from ipa group");
                            }
                            false => {
                                tracing::info!("Failed to remove user {user} from group {group}, may already not be in group?")
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("SyncVpnGroupsTask").versioned(1)
    }

    fn summarize(&self, id: ID) -> String {
        format!("[{id} | sync vpn task]")
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(120.0)
    }

    fn retry_count(&self) -> usize {
        0
    }
}
