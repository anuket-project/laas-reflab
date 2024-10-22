//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::tracing;
use config::settings;
use dal::{new_client, AsEasyTransaction, FKey};
use models::{
    dashboard::{Aggregate, Instance, StatusSentiment},
    inventory::Host,
    EasyLog,
};
use notifications::email::{send_to_admins_email, send_to_admins_gchat};
use serde::{self, Deserialize, Serialize};
use tascii::prelude::*;

use crate::{
    deploy_booking::{
        configure_networking::ConfigureNetworking, manage_eve_nodes::DeleteEveNode,
        net_config::empty_network_config, set_host_power_state::SetPower,
    },
    resource_management::ipmi_accounts::DeleteIPMIAccount,
    retry_for,
};

tascii::mark_task!(CleanupHost);
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct CleanupHost {
    pub agg_id: FKey<Aggregate>,
    pub instance: FKey<Instance>,
    pub host_id: FKey<Host>,
}

impl AsyncRunnable for CleanupHost {
    type Output = ();

    async fn run(
        &mut self,
        context: &tascii::prelude::Context,
    ) -> Result<Self::Output, tascii::prelude::TaskError> {
        self.instance
            .log(
                "Shutting Down Host",
                "host is being powered down to save energy",
                StatusSentiment::InProgress,
            )
            .await;

        retry_for(SetPower::off(self.host_id), context, 10, 10).expect("couldn't power down host");

        self.instance
            .log(
                "Removing IPMI Accounts",
                "host IPMI accounts are being deleted",
                StatusSentiment::InProgress,
            )
            .await;

        context.spawn(DeleteIPMIAccount {
            host: self.host_id,
            userid: "4".to_owned(),
        });

        self.instance
            .log(
                "Tearing Down Networks",
                "network access for this host is being removed",
                StatusSentiment::InProgress,
            )
            .await;

        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let nets_jh = context.spawn(ConfigureNetworking {
            net_config: empty_network_config(self.host_id, &mut transaction).await,
        });

        let _ignore = nets_jh.join();

        let agg = self
            .agg_id
            .get(&mut transaction)
            .await
            .expect("Expected to find aggregate");
        let host = self
            .host_id
            .get(&mut transaction)
            .await
            .expect("Expected to find host");

        let image_id = self
            .instance
            .get(&mut transaction)
            .await
            .expect("Expected to get instance")
            .config
            .image;
        if image_id
            .get(&mut transaction)
            .await
            .expect("Expected to find image name")
            .name
            .to_lowercase()
            .contains("eve")
        {
            match context
                .spawn(DeleteEveNode {
                    host_id: self.host_id,
                })
                .join()
            {
                Ok(_) => {
                    self.instance
                        .log(
                            "Cleanup Finished",
                            "host has been deprovisioned",
                            StatusSentiment::Succeeded,
                        )
                        .await;
                }
                Err(e) => {
                    tracing::error!("Failed to delete host {} from sandbox due to error: {e:?}. Aggregate is {:?}, owned by {}", host.server_name.clone(), self.agg_id, match agg.metadata.owner.clone() {
                        Some(s) => s,
                        None => "nobody".to_owned()
                    });

                    // send_to_admins_gchat(format!(
                    //     "Failed to delete {} from sandbox due to error: {e:?}",
                    //     host.server_name.clone()
                    // ))
                    // .await;
                    // send_to_admins_email(format!(
                    //     "Failed to delete {} from sandbox due to error: {e:?}",
                    //     host.server_name.clone()
                    // ))
                    // .await;

                    self.instance
                        .log(
                            "Failed to offboard node from sandbox",
                            "host has been deprovisioned, but not offboarded from sandbox",
                            StatusSentiment::Succeeded,
                        )
                        .await;
                }
            }
        }

        Ok(())
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("CleanHostTask").versioned(1)
    }
}
