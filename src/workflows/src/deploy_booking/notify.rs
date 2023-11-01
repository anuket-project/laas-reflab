//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use config::{settings, Situation};
use models::{
    dal::{new_client, AsEasyTransaction, FKey, ID},
    dashboard::Aggregate,
};
use notifications::{booking_ended, booking_ending, booking_started, BookingInfo, Env};
use tascii::{prelude::*, task_trait::AsyncRunnable};

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct Notify {
    pub aggregate: FKey<Aggregate>,
    pub situation: Situation,
}

#[derive(Debug, Clone, Copy, Hash, Serialize, Deserialize)]
pub enum NotifyBookingStatus {
    BookingDeployed,
}

tascii::mark_task!(Notify);
impl AsyncRunnable for Notify {
    type Output = ();

    async fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.expect("Expected to connect to db");
        let mut transaction = client
            .easy_transaction()
            .await
            .expect("Transaction creation error");

        let agg = self.aggregate.get(&mut transaction).await.unwrap();
        let env = Env {
            project: agg.origin.clone(),
        };
        let info = BookingInfo {
            owner: agg.metadata.owner.clone().unwrap_or("None".to_owned()),
            collaborators: agg.users.clone(),
            lab: agg.metadata.lab.clone().unwrap_or("None".to_owned()),
            id: agg.metadata.booking_id.clone().unwrap_or("None".to_owned()),
            template: agg
                .template
                .get(&mut transaction)
                .await
                .unwrap()
                .name
                .clone(),
            purpose: agg.metadata.purpose.clone().unwrap_or("None".to_owned()),
            start_date: agg.metadata.start,
            end_date: agg.metadata.end,
            dashboard_url: match Some(agg.origin.clone()) {
                Some(p) => settings()
                    .projects
                    .get(p.as_str())
                    .unwrap()
                    .dashboard_url
                    .clone(),
                None => "None".to_owned(),
            },
            configuration: agg.configuration.clone(),
        };

        transaction.commit().await.unwrap();

        match self.situation {
            Situation::BookingCreated => {
                booking_started(&env, &info)
                    .await
                    .expect("couldn't notify users");
            }
            Situation::BookingExpired => booking_ended(&env, &info)
                .await
                .expect("couldn't notify users"),
            Situation::BookingExpiring => booking_ending(&env, &info)
                .await
                .expect("couldn't notify users"),
            _ => todo!(),
        }

        Ok(())
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("send_notifications").versioned(1)
    }

    fn retry_count(&self) -> usize {
        0
    }
}
