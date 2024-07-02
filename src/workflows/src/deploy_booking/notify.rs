//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::collections::HashMap;

use common::prelude::{chrono};
use config::{settings, Situation};
use models::{
    dal::{new_client, AsEasyTransaction, FKey},
    dashboard::Aggregate,
};
use notifications::{
    booking_ended, booking_ending, booking_started, collaborator_added, request_booking_extension, BookingInfo, Env
};
use tascii::{prelude::*, task_trait::AsyncRunnable};

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct Notify {
    pub aggregate: FKey<Aggregate>,
    pub situation: Situation,
    pub extra_context: Vec<(String, String)>
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
            project: agg
                .lab
                .get(&mut transaction)
                .await
                .expect("Expected to find lab")
                .name
                .clone(),
            //project: agg.metadata.project.clone().unwrap_or("None".to_owned()),
        };

        let context_map: HashMap<String, String> = HashMap::from_iter(self.extra_context.clone().into_iter());
        let info = BookingInfo {
            owner: agg.metadata.owner.clone().unwrap_or("None".to_owned()),
            collaborators: agg
                .users
                .iter()
                .filter(|&username| *username != agg.metadata.owner.as_deref().unwrap_or_default())
                .cloned()
                .collect(),
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
            project: agg.metadata.project.clone().unwrap_or("None".to_owned()),
            start_date: agg.metadata.start,
            end_date: match context_map.get("ending_override") {
                Some(o) => {
                    match chrono::DateTime::parse_from_rfc2822(&o.to_string()) {
                        Ok(parsed) => {
                            Some(parsed.with_timezone(&chrono::Utc))
                        },
                        Err(_) => agg.metadata.end
                    }
                },
                None => agg.metadata.end
            },
            dashboard_url: match Some(agg.lab) {
                Some(p) => settings()
                    .projects
                    .get(
                        p.get(&mut transaction)
                            .await
                            .expect("Expected to find lab")
                            .name
                            .clone()
                            .as_str(),
                    )
                    .unwrap()
                    .dashboard_url
                    .clone(),
                None => "None".to_owned(),
            },
            configuration: agg.configuration.clone(),
        };

        transaction.commit().await.unwrap();

        match self.situation.clone() {
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
            Situation::CollaboratorAdded(users) => collaborator_added(&env, &info, users)
                .await
                .expect("couldn't notify users"),
            Situation::RequestBookingExtension => request_booking_extension(
                &env,
                &info,
                context_map.get("extension_date").unwrap_or(&format!("N/A")),
                context_map.get("extension_reason").unwrap_or(&format!("N/A")))
                .await
                .expect("couldn't notify admins"),
            _ => todo!()
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
