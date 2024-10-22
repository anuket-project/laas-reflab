use common::prelude::*;
use dal::*;
use models::{dashboard::Aggregate, inventory::Lab};
use serde::{Deserialize, Serialize};
use tascii::task_trait::{AsyncRunnable, TaskIdentifier};
use users::ipa;

tascii::mark_task!(SyncVPN);
#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub struct SyncVPN {
    pub users: Vec<String>,
}

impl AsyncRunnable for SyncVPN {
    type Output = ();

    async fn run(
        &mut self,
        _context: &tascii::prelude::Context,
    ) -> Result<Self::Output, tascii::prelude::TaskError> {
        let mut client = new_client().await.expect("Expected to connect to db");
        let mut transaction = client
            .easy_transaction()
            .await
            .expect("Transaction creation error");

        let mut ipa = ipa::IPA::init().await.expect("Expected to connect to IPA");

        let managed_groups: Vec<String> = config::settings()
            .projects
            .keys()
            .map(|p| p.to_owned())
            .collect();

        for user in &self.users {
            match sync_vpn_for_user(user, managed_groups.clone(), &mut ipa, &mut transaction).await
            {
                Ok(results) => {
                    tracing::info!("Successfully updated VPN groups for {user}\nGroups added: {:?}\nGroups removed: {:?}", results.0, results.1);
                }
                Err(error) => {
                    return Err(tascii::prelude::TaskError::Reason(format!("{error:?}")));
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

/// Adds and removes groups from the IPA account of a user based on "Active" and "New" aggregates.
/// Groups will only be removed or added if they are considered to be "managed groups".
/// A group is managed if and only if it is listed as a project in config.yaml.
/// Managed groups will typically be the lab / project name. This prevents liblaas from removing unrelated groups from the IPA account.
/// If successful, this function will return a tuple containing two lists. The first list is the groups that were added. The second list is the groups that were removed.
/// If unsuccessfuly for any reason, it returns an error.
async fn sync_vpn_for_user(
    user: &String,
    managed_groups: Vec<String>,
    ipa: &mut ipa::IPA,
    transaction: &mut EasyTransaction<'_>,
) -> Result<(Vec<String>, Vec<String>), anyhow::Error> {
    let active_groups: Vec<String> = ipa
        .group_find_user(user)
        .await?
        .iter()
        .filter(|&g| managed_groups.contains(g))
        .cloned()
        .collect();
    let correct_groups: Vec<String> = correct_groups_for_user(transaction, user)
        .await?
        .iter()
        .filter(|&g| managed_groups.contains(g))
        .cloned()
        .collect();

    let mut added_groups: Vec<String> = vec![];
    let mut removed_groups: Vec<String> = vec![];

    for group in &active_groups {
        if !correct_groups.contains(group) {
            println!("Removing {} from {} group", user, group);
            removed_groups.push(group.clone());
            ipa.group_remove_user(group, user).await?;
        }
    }

    for group in &correct_groups {
        if !active_groups.contains(group) {
            println!("Adding {} to {} group", user, group);
            added_groups.push(group.clone());
            ipa.group_add_user(group, user).await?;
        }
    }

    Ok((added_groups, removed_groups))
}

/// Public facing function for syncing the VPN groups of a single user
/// Create an ipa instance and transaction that are passed to the private function
pub async fn single_vpn_sync_for_user(
    user: &String,
) -> Result<(Vec<String>, Vec<String>), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let mut ipa = ipa::IPA::init().await.expect("Expected to connect to IPA");

    let managed_groups: Vec<String> = config::settings()
        .projects
        .keys()
        .map(|p| p.to_owned())
        .collect();

    sync_vpn_for_user(user, managed_groups, &mut ipa, &mut transaction).await
}

/// Finds the IPA groups that a user should be in based off of their active or new aggregates
async fn correct_groups_for_user(
    t: &mut EasyTransaction<'_>,
    username: &String,
) -> Result<Vec<String>, anyhow::Error> {
    let agg_tn = Aggregate::table_name();
    let lab_tn = Lab::table_name();

    let query = format!("select distinct {lab_tn}.name as group from {agg_tn} join {lab_tn} on {lab_tn}.id = {agg_tn}.lab where ({agg_tn}.lifecycle_state::varchar = '\"Active\"' or {agg_tn}.lifecycle_state::varchar = '\"New\"') and $1 = any(users);");
    match t.query(&query, &[&username]).await {
        Ok(rows) => Ok(rows.into_iter().map(|row| row.get("group")).collect()),
        Err(e) => Err(anyhow::Error::msg(e.to_string())),
    }
}

