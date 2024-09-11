//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

//! This module should probably actually be removed, if liblaas
//! can carry around a ref to the runtime all the time then it can
//! directly spawn in tasks and that simplifies control flow
//!
//! this mod does mean more clear separation and looser coupling,
//! but I don't think it's strictly worth it when the entire project
//! is as small as it is (relatively speaking)

use std::sync::Mutex;

use config::Situation;
use models::{
    dal::{new_client, AsEasyTransaction, FKey, ID},
    dashboard::{Aggregate, Instance},
    inventory::Host,
};

use models::inventory;

use common::prelude::{anyhow, crossbeam_channel, once_cell};

use crossbeam_channel::{Receiver, Sender};
use models::dal::web::*;

use tascii::prelude::*;

use crate::deploy_booking::{deploy_host::DeployHost, notify::Notify};

//use crate::actions::{Action, ActionID, StatusHandle};

//use crate::actions::*;

static ACTION_LOG_LOCK: Mutex<()> = Mutex::new(());

pub enum Action {
    DeployBooking {
        agg_id: FKey<Aggregate>,
    },
    CleanupBooking {
        agg_id: FKey<Aggregate>,
    },
    AddUsers {
        agg_id: FKey<Aggregate>,
        users: Vec<String>,
    },
    Reimage {
        host_id: FKey<Host>,
        inst_id: FKey<Instance>,
        agg_id: FKey<Aggregate>,
    },
    NotifyTask {
        agg_id: FKey<Aggregate>,
        situation: Situation,

        // List of (key, value) for extra items to be rendered in the template
        // Needs to be a Vec and not a map because task fields need to derive Hash
        // This was done in an attempt to generify notifications into a single task
        // Check the Notify task's run method to see expected fields for a template
        context: Vec<(String, String)>,
    },
    // UpdateUser { agg_id: LLID, user: dashboard::UserData },
    // RemoveUser { agg_id: LLID, user: i64 },
    // AddInstance { agg_id: LLID, instance: dashboard::InstanceData },
    // RemoveInstance { agg_id: LLID, instance: dashboard::InstanceData },
}

pub struct Dispatcher {
    rt: &'static Runtime,
}

//static ref DISPATCH: Sender<Action>;

pub static DISPATCH: once_cell::sync::OnceCell<Sender<Action>> = once_cell::sync::OnceCell::new();
// DISPATCH.get().unwrap().send(Action::DeployBooking { agg_id: <something> });

impl Dispatcher {
    pub fn init(rt: &'static Runtime) {
        let (s, r) = crossbeam_channel::unbounded();

        let d = Self { rt };

        std::thread::spawn(|| {
            d.handler(r);
        });

        DISPATCH.set(s).expect("dispatcher was already initialized");
    }

    pub fn handler(self, recv: Receiver<Action>) {
        while let Ok(v) = recv.recv() {
            let task: RunnableHandle = match v {
                Action::DeployBooking { agg_id } => crate::deploy_booking::BookingTask {
                    aggregate_id: agg_id,
                }
                .into(),
                Action::CleanupBooking { agg_id } => {
                    crate::cleanup_booking::CleanupAggregate { agg_id }.into()
                }
                Action::AddUsers { agg_id, users } => {
                    crate::users::AddUsers { agg_id, users }.into()
                }
                Action::Reimage {
                    agg_id,
                    inst_id,
                    host_id,
                } => DeployHost {
                    host_id,
                    aggregate_id: agg_id,
                    using_instance: inst_id,
                    distribution: None,
                }
                .into(),
                Action::NotifyTask {
                    agg_id,
                    situation,
                    context,
                } => Notify {
                    aggregate: agg_id,
                    situation,
                    extra_context: context,
                }
                .into(), // Action::UpdateUser { agg_id, user } => {
                         //     // TODO: Create task
                         //     let task_id: LLID = self.rt.enroll(todo!());
                         // },
                         // Action::RemoveUser { agg_id, user } => {
                         //     // TODO: Create task
                         //     let task_id: LLID = self.rt.enroll(todo!());
                         //     self.rt.set_target(task_id);
                         // },
                         // Action::AddInstance { agg_id, instance } => {
                         //     // TODO: Create task
                         //     let task_id: LLID = self.rt.enroll(todo!());
                         //     self.rt.set_target(task_id);
                         // },
                         // Action::RemoveInstance { agg_id, instance } => {
                         //     // TODO: Create task
                         //     let task_id: LLID = self.rt.enroll(todo!());
                         //     self.rt.set_target(task_id);
                         // },
            };

            let task_id = self.rt.enroll(task);
            self.rt.set_target(task_id);
        }
    }

    async fn set_depends(
        &self,
        id: ID,
        resources: Vec<FKey<inventory::Host>>,
    ) -> Result<(), anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;

        /*let options = TransactionOptions::builder()
        .read_concern(ReadConcern::majority())
        .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
        .build();*/

        //let coll = Host::all_hosts(&mut client).unwrap();

        for resource in resources {
            let _ = inventory::Action::add_for_host(&mut transaction, resource, false, id)
                .await
                .log_server_error("couldn't set depends for an action", true);
        }
        transaction.commit().await.unwrap();
        Ok(())
    }
}
