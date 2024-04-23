//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use models::{
    dal::{new_client, AsEasyTransaction, FKey},
    dashboard::{Aggregate, EasyLog, Instance, StatusSentiment},
    inventory::Host,
};
use serde::{self, Deserialize, Serialize};
use tascii::prelude::*;

use crate::{
    deploy_booking::{
        configure_networking::ConfigureNetworking, net_config::empty_network_config,
        set_host_power_state::SetPower,
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
                StatusSentiment::in_progress,
            )
            .await;

        retry_for(SetPower::off(self.host_id), context, 10, 10).expect("couldn't power down host");

        self.instance
            .log(
                "Removing IPMI Accounts",
                "host IPMI accounts are being deleted",
                StatusSentiment::in_progress,
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
                StatusSentiment::in_progress,
            )
            .await;

        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let nets_jh = context.spawn(ConfigureNetworking {
            net_config: empty_network_config(self.host_id, &mut transaction).await,
        });

        let _ignore = nets_jh.join();

        self.instance
            .log(
                "Cleanup Finished",
                "host has been deprovisioned",
                StatusSentiment::succeeded,
            )
            .await;

        Ok(())
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("CleanHostTask").versioned(1)
    }
}
