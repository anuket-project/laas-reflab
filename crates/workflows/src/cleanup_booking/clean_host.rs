use anyhow::{Context, Result};
use chrono::Utc;
use dal::{new_client, AsEasyTransaction, FKey};
use metrics::{BookingExpiredMetric, MetricHandler};
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
        match self.clone().send_expired_metric().await {
            Ok(_) => tracing::info!("Booking expired metric sent successfully."),
            Err(e) => tracing::error!("Failed to send booking expired metric: {:?}", e),
        }

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

impl CleanupHost {
    pub async fn send_expired_metric(self) -> Result<(), anyhow::Error> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let aggregate = self
            .agg_id
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner();

        let owner = aggregate
            .metadata
            .owner
            .ok_or_else(|| anyhow::anyhow!("Missing booking owner"))?;

        let booking_id = aggregate
            .metadata
            .booking_id
            .ok_or_else(|| anyhow::anyhow!("Missing booking_id"))?
            .parse::<i32>()
            .context("Failed to parse booking_id")?;

        let project = aggregate
            .metadata
            .project
            .ok_or_else(|| anyhow::anyhow!("Missing project"))?;

        let lab = aggregate
            .metadata
            .lab
            .ok_or_else(|| anyhow::anyhow!("Missing lab"))?;

        // Compute booking length from start to now
        let total_booking_length_days = aggregate
            .metadata
            .start
            .map(|start| {
                let now = Utc::now();
                now.signed_duration_since(start).num_days() as i32
            })
            .unwrap_or(0);

        let booking_expired_metric = BookingExpiredMetric {
            owner,
            booking_id,
            project,
            lab,
            mock: false,
            total_booking_length_days,
            ..Default::default()
        };

        MetricHandler::send(booking_expired_metric)
            .map_err(|e| anyhow::anyhow!("Failed to send booking expired metric: {:?}", e))?;

        Ok(())
    }
}
