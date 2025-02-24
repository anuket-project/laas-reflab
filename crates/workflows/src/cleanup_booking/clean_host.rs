use dal::{new_client, AsEasyTransaction, FKey};
use models::{
    dashboard::{Aggregate, Instance, StatusSentiment},
    inventory::Host,
    EasyLog,
};
use serde::{self, Deserialize, Serialize};
use tascii::prelude::*;

use crate::{
    configure_networking::{empty_network_config, ConfigureNetworking},
    deploy_booking::set_host_power_state::SetPower,
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

        Ok(())
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("CleanHostTask").versioned(1)
    }
}
