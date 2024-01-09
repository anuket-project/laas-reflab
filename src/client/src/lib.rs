//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

#![feature(
    result_flattening,
    let_chains,
    panic_backtrace_config,
    update_panic_count,
    panic_can_unwind
)]

pub mod importing;
pub mod mgmt_workflows;
pub mod remote;

use std::io::Write;

use crate::{importing::*, mgmt_workflows::BootToDev};

use common::prelude::{
    anyhow,
    config::{settings, Situation},
    inquire::validator::Validation,
    itertools::Itertools,
    serde_json, tracing,
};
use liblaas::{
    booking::make_aggregate,
    web::api::{self, BookingMetadataBlob},
};

use mgmt_workflows::BootBookedHosts;
// use mgmt_workflows::BootToNetwork;
use models::{
    allocation::{Allocation, ResourceHandle},
    dal::{new_client, AsEasyTransaction, DBTable, EasyTransaction, ExistingRow, FKey, ID},
    dashboard::{
        Aggregate, BookingMetadata, Instance, LifeCycleState, Network, ProvisionLogEvent, Template,
    },
    inventory::{BootTo, Host, Vlan},
};
use notifications::{
    email::{send_to_admins_email, send_to_admins_gchat},
    send_test_email,
};
use remote::{Password, Select, Server, Text};
use std::{collections::HashMap, fmt::Formatter, path::PathBuf, str::FromStr, time::Duration};
use tascii::prelude::Runtime;
use users::ipa::{UserData, *};
use workflows::{
    deploy_booking::{
        deploy_host::DeployHost,
        notify::Notify,
        set_host_power_state::{get_host_power_state, PowerState, SetPower},
    },
    entry::DISPATCH,
    resource_management::{allocator, mailbox::Mailbox},
};

/// Runs the cli
#[derive(Debug, Copy, Clone)]
pub enum LiblaasStateInstruction {
    ShutDown(),
    DoNothing(),
    ExitCLI(),
}

pub async fn cli_entry(
    tascii_rt: &'static Runtime,
    mut session: &Server,
) -> Result<LiblaasStateInstruction, anyhow::Error> {
    // we want panics within CLI to appear within stdout
    //tascii::set_capture_panics(false);
    // Set renderer
    //set_global_render_config(get_render_config());

    // Loop cli so users can do multiple things
    loop {
        let task: Result<&str, _> = Select::new("Task:", get_tasks(session))
            .with_help_message("Select a task to perform")
            .prompt(session);

        tracing::info!("Got resp from task selection: {task:?}");

        match task.expect("Expected task array to be non-empty") {
            // General interactions
            //"Test LibLaaS" => tests().await,
            "Recovery" => {
                match Select::new("select a recovery action: ", vec!["boot booked hosts"])
                    .prompt(session)?
                {
                    "boot booked hosts" => {
                        areyousure(session)?;

                        let task = BootBookedHosts {};

                        let id = tascii_rt.enroll(task.into());
                        tascii_rt.set_target(id);
                    }
                    _ => unreachable!(),
                }
            }
            "Use database" => use_database(session).await?,
            "Use IPA" => {
                use_ipa(session).await.expect("couldn't finish use ipa");
            }
            // Booking functions
            "Create booking" => create_booking(session)
                .await
                .expect("couldn't create booking"), // Dispatches booking creation
            "Expire booking" => expire_booking(session).await, // Dispatches the cleanup task
            "Extend booking" => extend_booking(session).await, // Will need to poke dashboard
            "Regenerate Booking C-I files" => regenerate_ci_files(session).await,
            "Rerun Cleanup" => {
                let id = Text::new("Enter UUID for cleanup task to rerun: ").prompt(session)?;

                areyousure(session)?;

                let uid = FKey::from_id(ID::from_str(&id).unwrap());
                DISPATCH
                    .get()
                    .unwrap()
                    .send(workflows::entry::Action::CleanupBooking { agg_id: uid })?;
                let _ = writeln!(session, "Successfully started cleanup");
            }

            "Manage Templates" => modify_templates(session).await,

            "Rerun Deploy" => {
                let id = Text::new("Enter UUID for aggregate to rerun: ").prompt(session)?;

                areyousure(session)?;

                let uid = FKey::from_id(ID::from_str(&id).unwrap());
                DISPATCH
                    .get()
                    .unwrap()
                    .send(workflows::entry::Action::DeployBooking { agg_id: uid })?;
                let _ = writeln!(session, "Successfully started deploy");
            }

            "Overrides" => overrides(session, tascii_rt).await?,

            "Query" => query(session).await,

            "Import" => {
                import(session).await.expect("Failed to import");
            }
            // Get useful info
            "Get Usage Data" => {
                get_usage_data(session).await;
            }
            "Run Migrations" => {
                models::dal::initialize().await.unwrap();
            }
            "Restart CLI" => return Ok(LiblaasStateInstruction::DoNothing()),
            "Shut Down Tascii" => {
                areyousure(session)?;
                return Ok(LiblaasStateInstruction::ShutDown());
            }
            "Exit CLI" => return Ok(LiblaasStateInstruction::ExitCLI()),
            &_ => {}
        }

        //tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn modify_templates(session: &Server) {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let action = Select::new("select an action: ", vec!["set template public/private"])
        .prompt(session)
        .unwrap();

    match action {
        "set template public/private" => {
            let template = select_template(session, &mut transaction).await.unwrap();
            let status = Select::new("set template to: ", vec!["public", "private"])
                .prompt(session)
                .unwrap();

            let public = match status {
                "public" => true,
                "private" => false,
                _ => unreachable!(),
            };

            let mut template = template.get(&mut transaction).await.unwrap();

            template.public = public;

            template.update(&mut transaction).await.unwrap();
        }
        _ => unreachable!(),
    }

    transaction.commit().await.unwrap();
}

async fn summarize_aggregate(
    mut session: &Server,
    transaction: &mut EasyTransaction<'_>,
    agg_id: FKey<Aggregate>,
) {
    let agg = agg_id.get(transaction).await.unwrap().into_inner();

    let _allocations = Allocation::all_for_aggregate(transaction, agg.id)
        .await
        .unwrap();

    let _ = writeln!(session, "===== Aggregate by id {:?}", agg.id);

    let BookingMetadata {
        booking_id,
        owner,
        lab,
        purpose,
        project,
        start,
        end,
    } = agg.metadata.clone();

    let _ = writeln!(session, "Booking ID: {booking_id:?}");
    let _ = writeln!(session, "Purpose: {purpose:?}");
    let _ = writeln!(session, "Owned by: {owner:?}");
    let _ = writeln!(session, "Start: {start:?}");
    let _ = writeln!(session, "End: {end:?}");
    let _ = writeln!(session, "Lab: {lab:?}");
    let _ = writeln!(session, "Project: {project:?}");

    let _ = writeln!(session, "Collaborators:");
    for user in agg.users.iter() {
        let _ = writeln!(session, "- {user}");
    }

    let _ = writeln!(session, "Networks:");
    for (net, vlan) in agg
        .vlans
        .get(transaction)
        .await
        .unwrap()
        .into_inner()
        .networks
    {
        let net = net.get(transaction).await.unwrap().into_inner();
        let vlan = vlan.get(transaction).await.unwrap().into_inner();

        let Network {
            id: _n_id,
            name,
            public: _,
        } = net;
        let Vlan {
            id: _v_id,
            vlan_id,
            public_config: _,
        } = vlan;

        let _ = writeln!(session, "- {name} with assigned vlan {vlan_id}");
    }

    let _ = writeln!(session, "Resources:");
    for instance in agg.instances(transaction).await.unwrap() {
        let instance = instance.into_inner();

        let Instance {
            id: _,
            metadata: _,
            aggregate: _,
            within_template: _,
            config,
            network_data: _,
            linked_host,
        } = instance;

        let host = match linked_host {
            Some(h) => {
                let h = h.get(transaction).await.unwrap().into_inner();
                format!("{}", h.server_name)
            }
            None => {
                let inst_of = config.flavor.get(transaction).await.unwrap().name.clone();
                format!("<unassigned host of type {inst_of}>")
            }
        };

        let config = {
            let hn = config.hostname;
            let img = config.image.get(transaction).await.unwrap().into_inner();
            let img_name = img.name;
            let img_cname = img.cobbler_name;

            format!("{{ hostname {hn}, image {img_name} which in cobbler is {img_cname} }}")
        };

        let _ = writeln!(session, "- Host {host} with config {config}");
        let _ = writeln!(session, "  - Log events:");
        let mut events = ProvisionLogEvent::all_for_instance(transaction, instance.id)
            .await
            .unwrap_or(vec![]);
        events.sort_by_key(|e| e.time);
        for ev in events {
            let time = ev.time.to_rfc2822();
            let content = ev.prov_status.to_string();
            let _ = writeln!(session, "    - {time}: {content}");
            //writeln!(session, "    - {}", ev.to_str
        }
    }
    let _ = writeln!(session, "=========\n");
}

async fn summarize_host(transaction: &mut EasyTransaction<'_>, host: FKey<Host>) -> String {
    let host = host.get(transaction).await.unwrap();

    format!("{}", host.server_name)
}

async fn get_host_by_hostname(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> (Host, ResourceHandle) {
    let hostname = Text::new("hostname:").prompt(session).unwrap();
    let resource = Host::get_by_name(transaction, hostname)
        .await
        .expect("no host found by that hostname")
        .into_inner();
    let handle = ResourceHandle::handle_for_host(transaction, resource.id)
        .await
        .expect("host didn't have a resource handle");

    (resource, handle)
}

async fn query(mut session: &Server) {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    match Select::new(
        "Select something to query:",
        vec![
            "aggregate for host",
            "config for host",
            "summarize current bookings",
            "ipmi creds for host",
            "list free hosts",
            "list free vlans",
            "get host power state",
        ],
    )
    .prompt(session)
    .unwrap()
    {
        "get host power state" => {
            let host = select_host(session, &mut transaction).await.unwrap();
            let host = host.get(&mut transaction).await.unwrap();

            let ipmi_fqdn = &host.ipmi_fqdn;
            let ipmi_admin_user = &host.ipmi_user;
            let ipmi_admin_password = &host.ipmi_pass;

            let ps = get_host_power_state(ipmi_fqdn, ipmi_admin_user, ipmi_admin_password);

            let _ = writeln!(session, "Host is in state {ps:?}");
        }
        "list free vlans" => {
            let mut vlans = allocator::Allocator::instance()
                .get_free_vlans(&mut transaction)
                .await
                .unwrap();

            vlans.sort_by_key(|v| v.0.vlan_id);

            let _ = writeln!(session, "Free vlans:");
            for (vlan, _) in vlans {
                let vid = vlan.vlan_id;
                let id = vlan.id.into_id();
                let pc = vlan.public_config.clone();
                let public = if pc.is_some() { "public" } else { "private" };

                let _ = writeln!(session, "- {id} | vlan id {vid} | {public}");
            }
            let _ = writeln!(session, "=====");
        }
        "list free hosts" => {
            let hosts = allocator::Allocator::instance()
                .get_free_hosts(&mut transaction)
                .await
                .unwrap();

            let _ = writeln!(session, "Free hosts:");
            for (host, _) in hosts {
                let hs = summarize_host(&mut transaction, host.id).await;

                let _ = writeln!(session, "- {hs}");
            }
            let _ = writeln!(session, "=====");
        }
        "ipmi creds for host" => {
            let (host, _handle) = get_host_by_hostname(session, &mut transaction).await;

            let _ = writeln!(
                session,
                "IPMI (FQDN, User, Pass, MAC):\n{}\n{}\n{}\n{}",
                host.ipmi_fqdn, host.ipmi_user, host.ipmi_pass, host.ipmi_mac
            );
        }
        "summarize current bookings" => {
            let state = Select::new(
                "Get bookings in state:",
                vec![
                    LifeCycleState::New,
                    LifeCycleState::Active,
                    LifeCycleState::Done,
                ],
            )
            .prompt(session)
            .unwrap();

            let aggregates = Aggregate::select()
                .where_field("lifecycle_state")
                .equals(state)
                .run(&mut transaction)
                .await
                .unwrap();

            for agg in aggregates {
                summarize_aggregate(session, &mut transaction, agg.id).await;
            }
        }
        "aggregate for host" => {
            let hostname = Text::new("hostname:").prompt(session).unwrap();
            let resource = Host::get_by_name(&mut transaction, hostname)
                .await
                .expect("no host found by that hostname")
                .into_inner();
            let handle = ResourceHandle::handle_for_host(&mut transaction, resource.id)
                .await
                .expect("host didn't have a resource handle");

            let current_allocations = Allocation::find(&mut transaction, handle.id, false)
                .await
                .unwrap();

            match current_allocations.as_slice() {
                [] => {
                    let _ = writeln!(session, "Host is not currently a member of an allocation");
                }
                [one] => {
                    let fa = one.for_aggregate;
                    let a = one.id;

                    let _ = writeln!(
                        session,
                        "Host is within allocation {a:?}, which is part of aggregate {fa:?}"
                    );

                    if let Some(aid) = fa {
                        summarize_aggregate(session, &mut transaction, aid).await;
                    }
                }
                more => {
                    unreachable!("Host was a member of multiple allocations, they are {more:?}, which is a DB integrity issue!")
                }
            }
        }
        "config for host" => {
            let hostname = Text::new("hostname:").prompt(session).unwrap();
            let resource = Host::get_by_name(&mut transaction, hostname)
                .await
                .expect("no host found by that hostname")
                .into_inner();
            let handle = ResourceHandle::handle_for_host(&mut transaction, resource.id)
                .await
                .expect("host didn't have a resource handle");

            let allocation = Allocation::find(&mut transaction, handle.id, false)
                .await
                .unwrap()
                .get(0)
                .unwrap()
                .clone();

            let agg = allocation
                .for_aggregate
                .unwrap()
                .get(&mut transaction)
                .await
                .unwrap()
                .into_inner();

            for inst in agg.instances(&mut transaction).await.unwrap() {
                let inst = inst.into_inner();
                if let Some(h) = inst.linked_host
                    && h == resource.id
                {
                    // found our host, now can look at config
                    let conf = inst.config;
                    let image = conf.image.get(&mut transaction).await.unwrap().into_inner();
                    let hostname = conf.hostname.clone();
                    let _ = writeln!(session, "Hostname {hostname}");
                    let _ = writeln!(
                        session,
                        "Assigned image: {}, cobbler id {}, id {:?}",
                        image.name, image.cobbler_name, image.id
                    );
                    let generated = workflows::deploy_booking::generate_cloud_config(
                        conf.clone(),
                        h,
                        inst.id,
                        agg.id,
                        &mut transaction,
                    )
                    .await
                    .unwrap();

                    let _ = writeln!(session, "Primary CI file:");
                    let _ = writeln!(session, "{generated}");
                    let _ = writeln!(session, "=======");

                    for cif in conf.cifile {
                        let cif = cif.get(&mut transaction).await.unwrap().into_inner();
                        let _ = writeln!(
                            session,
                            "Additional CI file {:?}, priority {}:",
                            cif.id, cif.priority
                        );
                        let _ = writeln!(session, "=== BEGIN CONFIG FILE ===");
                        let _ = writeln!(session, "{}", cif.data);
                        let _ = writeln!(session, "==== END CONFIG FILE ====");
                    }
                }
            }
        }
        _ => {}
    }
    let _ = transaction.commit().await;
}

fn areyousure(session: &Server) -> Result<(), anyhow::Error> {
    match Select::new("Are you sure? ", vec!["no", "yes"])
        .prompt(session)
        .unwrap()
    {
        "no" => Err(anyhow::Error::msg("user was not sure")),
        "yes" => Ok(()),
        _ => unreachable!(),
    }
}

async fn select_host(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Host>, anyhow::Error> {
    let hosts = Host::select().run(transaction).await?;

    let mut disps = Vec::new();

    impl std::fmt::Display for DispHost {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            let host = self.host.clone();
            write!(f, "{}", host.server_name)
        }
    }

    for host in hosts {
        disps.push(DispHost {
            host: host.into_inner(),
        })
    }

    Ok(Select::new("select a host: ", disps)
        .prompt(session)
        .unwrap()
        .host
        .id)
}

fn select_lifecyclestate(session: &Server) -> Result<LifeCycleState, anyhow::Error> {
    let state = Select::new(
        "Select a state for filtering aggregates:",
        vec![
            LifeCycleState::New,
            LifeCycleState::Active,
            LifeCycleState::Done,
        ],
    )
    .prompt(session)
    .unwrap();

    Ok(state)
}

async fn select_aggregate(
    session: &Server,
    from_state: LifeCycleState,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Aggregate>, anyhow::Error> {
    let aggs = Aggregate::select()
        .where_field("lifecycle_state")
        .equals(from_state)
        .run(transaction)
        .await?;

    let mut disps = Vec::new();

    for agg in aggs {
        let purpose = agg.metadata.purpose.clone().unwrap_or("<none>".to_owned());
        let owner = agg
            .metadata
            .owner
            .clone()
            .unwrap_or("<no owner>".to_owned());

        let mut dispinsts = Vec::new();

        for inst in agg.instances(transaction).await? {
            let inst = inst.into_inner();

            let di = DispInst::from_inst(transaction, inst).await;

            dispinsts.push(di);
        }

        disps.push(DispAgg {
            id: agg.id,
            owner,
            purpose,
            hosts: dispinsts,
        });
    }

    impl std::fmt::Display for DispAgg {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            let instances: String = self
                .hosts
                .clone()
                .into_iter()
                .map(|inst| format!("\n - {inst}"))
                .collect();

            write!(
                f,
                "{} -> {} (owned by {}), with instances {}",
                self.id.into_id(),
                self.purpose,
                self.owner,
                instances
            )
        }
    }

    Ok(Select::new("select an aggregate:", disps)
        .prompt(session)?
        .id)
}

async fn select_template(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Template>, anyhow::Error> {
    let temps = Template::select().run(transaction).await?;

    let mut disps = Vec::new();

    impl std::fmt::Display for DispTemplate {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            let temp = self.template.clone();
            write!(
                f,
                "{:?} | name: {}, owner: {:?}, public: {}, description: {}",
                temp.id.into_id(),
                temp.name,
                temp.owner,
                temp.public,
                temp.description
            )
        }
    }

    for temp in temps {
        disps.push(DispTemplate {
            template: temp.into_inner(),
        })
    }

    Ok(Select::new("select a template: ", disps)
        .prompt(session)
        .unwrap()
        .template
        .id)
}

#[derive(Clone)]
struct DispTemplate {
    template: Template,
}

#[derive(Clone)]
struct DispHost {
    host: Host,
}

#[derive(Clone)]
struct DispInst {
    id: FKey<Instance>,
    hostname: String,
    host: String,
}

#[derive(Clone)]
struct DispAgg {
    id: FKey<Aggregate>,
    purpose: String,
    owner: String,
    hosts: Vec<DispInst>,
}

impl DispInst {
    pub async fn from_inst(transaction: &mut EasyTransaction<'_>, inst: Instance) -> Self {
        let host = if let Some(h) = inst.linked_host {
            h.get(transaction).await.unwrap().server_name.clone()
        } else {
            "<unknown>".to_owned()
        };

        let hostname = inst.config.hostname.clone();

        let di = DispInst {
            id: inst.id,
            hostname,
            host,
        };

        di
    }
}

async fn select_instance(
    session: &Server,
    within_agg: FKey<Aggregate>,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Instance>, anyhow::Error> {
    let instances = within_agg
        .get(transaction)
        .await
        .unwrap()
        .instances(transaction)
        .await
        .unwrap();

    let mut disps = Vec::new();

    for inst in instances {
        let di = DispInst::from_inst(transaction, inst.into_inner()).await;
        disps.push(di);
    }

    impl std::fmt::Display for DispInst {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "{} -> {} ({})",
                self.id.into_id(),
                self.hostname,
                self.host
            )
        }
    }

    Ok(Select::new("select an instance:", disps)
        .prompt(session)?
        .id)
}

async fn overrides(mut session: &Server, tascii_rt: &'static Runtime) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    match Select::new(
        "Select an override to apply:",
        vec![
            "override aggregate state",
            "force release aggregate",
            "force reset and rerun aggregate",
            "boot host to",
            "rerun host deployment",
            "send notification",
            "send email to admins",
            "test email template",
            "send google chat to admins",
            "set host power state",
            "override endpoint hook",
        ],
    )
    .prompt(session)
    .unwrap()
    {
        "override endpoint hook" => {
            let lcs = select_lifecyclestate(session).unwrap();
            let agg = select_aggregate(session, lcs, &mut transaction)
                .await
                .unwrap();
            let inst = select_instance(session, agg, &mut transaction)
                .await
                .unwrap();
            let hook = Select::new(
                "enter hook name: ",
                Mailbox::live_hooks(inst).await.unwrap(),
            )
            .prompt(session)
            .unwrap();

            let hook = Mailbox::get_endpoint_hook(inst, &hook).await.unwrap();

            let msg = Text::new("Enter message for hook: ")
                .prompt(session)
                .unwrap();

            Mailbox::push(
                common::prelude::axum::extract::Path((hook.for_instance, hook.unique)),
                msg,
            )
            .await;

            println!("Pushed to the hook an override");
        }
        "set host power state" => {
            let host = select_host(session, &mut transaction).await.unwrap();

            let pstate = Select::new(
                "select a desired power state: ",
                vec![PowerState::On, PowerState::Off, PowerState::Reset],
            )
            .prompt(session)
            .unwrap();

            areyousure(session)?;

            let id = tascii_rt.enroll(SetPower { host, pstate }.into());

            tascii_rt.set_target(id);
        }
        "send email to admins" => {
            let email = Text::new("Message: ").prompt(session).unwrap();

            send_to_admins_email(email).await;
        }
        "test email template" => {
            let status = Select::new(
                "Select a Status: ",
                vec![
                    Situation::BookingCreated,
                    Situation::BookingExpiring,
                    Situation::BookingExpired,
                ],
            )
            .prompt(session)
            .unwrap();

            let project_type = Select::new(
                "Select a Project: ",
                settings()
                    .projects
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<&str>>(),
            )
            .prompt(session)
            .unwrap();
            let dest_user = Text::new("Destination Username: ").prompt(session).unwrap();

            let result = send_test_email(
                status,
                project_type.to_owned(),
                dest_user.clone(),
                Some(vec![dest_user.clone()]),
            )
            .await;

            match result {
                Ok(_) => {
                    tracing::info!("Successfully sent email to {:?}", &dest_user);
                }
                Err(e) => {
                    tracing::error!("Failed to send email to {:?}: {:?}", &dest_user, e);
                }
            }
        }

        "send google chat to admins" => {
            let gchat = Text::new("Message: ").prompt(session).unwrap();

            send_to_admins_gchat(gchat).await;
        }
        "rerun host deployment" => {
            let agg = select_aggregate(session, LifeCycleState::New, &mut transaction)
                .await
                .unwrap();

            let inst = select_instance(session, agg, &mut transaction)
                .await
                .unwrap();

            let host_id = inst.get(&mut transaction).await.unwrap().linked_host;

            let host_id = if let Some(id) = host_id {
                id
            } else {
                let _ = writeln!(session, "No linked host, so can't simply restart");
                panic!()
            };

            areyousure(session)?;

            let task = DeployHost {
                host_id,
                aggregate_id: agg,
                using_instance: inst,
            };

            let id = tascii_rt.enroll(task.into());
            tascii_rt.set_target(id);

            format!("Reran host deploy as task id {id}");
        }
        "override aggregate state" => {
            let agg_id = Text::new("aggregate to change state of:")
                .prompt(session)
                .unwrap();
            let agg_id = FKey::from_id(ID::from_str(agg_id.as_str()).unwrap());
            let mut agg: ExistingRow<Aggregate> = agg_id.get(&mut transaction).await.unwrap();

            let _ = writeln!(session, "Current state of agg is {:?}", agg.state);

            let new_state = Select::new(
                "Select a new state for the aggregate:",
                vec![
                    LifeCycleState::New,
                    LifeCycleState::Active,
                    LifeCycleState::Done,
                ],
            )
            .prompt(session)
            .unwrap();

            agg.state = new_state;

            agg.update(&mut transaction).await.unwrap();

            let _ = writeln!(
                session,
                "State updated to {new_state} on aggregate {agg_id:?}"
            );
        }
        "force reset and rerun aggregate" => {
            let agg_id = Text::new("aggregate to forcefully release and rerun:")
                .prompt(session)
                .unwrap();
            let agg_id = FKey::from_id(ID::from_str(agg_id.as_str()).unwrap());
            let mut agg: ExistingRow<Aggregate> = agg_id.get(&mut transaction).await.unwrap();

            areyousure(session)?;

            agg.state = LifeCycleState::New;
            agg.update(&mut transaction).await.unwrap();

            allocator::Allocator::instance()
                .deallocate_aggregate(&mut transaction, agg_id)
                .await
                .expect("couldn't dealloc agg");

            let _ = writeln!(
                session,
                "Released aggregate, deallocated hosts, now rerunning it..."
            );

            DISPATCH
                .get()
                .unwrap()
                .send(workflows::entry::Action::DeployBooking { agg_id: agg.id })
                .unwrap();

            let _ = writeln!(session, "Done starting rerun");
        }
        "force release aggregate" => {
            let agg_id = Text::new("aggregate to forcefully release:")
                .prompt(session)
                .unwrap();
            let agg_id = FKey::from_id(ID::from_str(agg_id.as_str()).unwrap());
            let mut agg: ExistingRow<Aggregate> = agg_id.get(&mut transaction).await.unwrap();

            let _user_is_sure = areyousure(session)?;

            agg.state = LifeCycleState::Done;

            agg.update(&mut transaction).await.unwrap();

            allocator::Allocator::instance()
                .deallocate_aggregate(&mut transaction, agg_id)
                .await
                .expect("couldn't dealloc agg");

            let _ = writeln!(session, "Released aggregate, deallocated hosts");
        }
        "boot host to" => {
            let hostname = Text::new("hostname:").prompt(session).unwrap();
            let bootdev = Select::new("select boot device:", vec![BootTo::Disk, BootTo::Network])
                .prompt(session)
                .unwrap();
            let resource = Host::get_by_name(&mut transaction, hostname)
                .await
                .expect("no host found by that hostname")
                .into_inner();

            let task = BootToDev {
                host: resource.id,
                bootdev,
            };

            let id = tascii_rt.enroll(task.into());
            tascii_rt.set_target(id);

            format!("Enrolled boot dev task as id {id:?}");
        }
        "send notification" => {
            let lcs = Select::new(
                "choose aggregate state from which to select an aggregate to notify about: ",
                vec![
                    LifeCycleState::New,
                    LifeCycleState::Active,
                    LifeCycleState::Done,
                ],
            )
            .prompt(session)
            .unwrap();

            let agg = select_aggregate(session, lcs, &mut transaction).await?;

            let situation = Select::new(
                "select situation to send notification for: ",
                vec![
                    Situation::BookingCreated,
                    Situation::BookingExpired,
                    Situation::BookingExpiring,
                    Situation::VPNAccessAdded,
                    Situation::VPNAccessRemoved,
                    Situation::PasswordResetRequested,
                ],
            )
            .prompt(session)?;

            let task = Notify {
                aggregate: agg,
                situation,
            };

            let id = tascii_rt.enroll(task.into());
            tascii_rt.set_target(id);

            let _ = writeln!(session, "Started notify task");
        }
        _ => unreachable!(),
    }

    transaction.commit().await?;

    Ok(())
}

async fn use_database(mut session: &Server) -> Result<(), anyhow::Error> {
    loop {
        let mut col_vec = get_collections(session);
        col_vec.push("Select new task".to_owned());
        let selected_col: Result<String, _> = Select::new("Collection:", col_vec)
            .with_help_message("Select a task to perform")
            .prompt(session);
        match selected_col
            .expect("Expected collection array to be non-empty")
            .as_str()
        {
            "Select new task" => {
                break Ok(());
            }
            &_ => loop {
                let mut db_int_vec = get_db_interactions();
                db_int_vec.push("Select new collection");
                let selected_op: Result<&str, _> = Select::new("Collection:", db_int_vec)
                    .with_help_message("Select a collection to modify")
                    .prompt(session);

                match selected_op.expect("Expected db operation array to be non-empty") {
                    "Select new collection" => {
                        break;
                    }
                    "Create" => {
                        writeln!(session, "creating!")?;
                    }
                    "Edit" => {
                        writeln!(session, "editing!")?;
                    }
                    "List" => {
                        writeln!(session, "listing!")?;
                    }
                    "Delete" => {
                        writeln!(session, "deleting!")?;
                    }
                    &_ => {}
                }
            },
        }
    }
}

async fn use_ipa(mut session: &Server) -> Result<(), common::prelude::anyhow::Error> {
    let mut ipa_instance = IPA::init()
        .await
        .expect("Expected to initialize IPA instance");
    loop {
        let mut ipa_vec = get_ipa_interactions();
        ipa_vec.push("Select new task");
        let selected_op: Result<&str, _> = Select::new("IPA Action:", ipa_vec)
            .with_help_message("Select an IPA interaction")
            .prompt(session);

        match selected_op.expect("Expected db operation array to be non-empty") {
            "Select new task" => {
                break Ok(());
            }
            "Create user" => {
                let new_user = User {
                    uid: Text::new("Enter uid:").prompt(session)?,
                    givenname: Text::new("Enter first name:").prompt(session)?,
                    sn: Text::new("Enter last name:").prompt(session)?,
                    cn: match Text::new("Enter full name:").prompt(session)?.as_str() {
                        "" => None,
                        s => Some(s.to_owned()),
                    },
                    homedirectory: match Text::new("Enter home dir:")
                        .with_validator(|p: &str| {
                            if p.eq("") | PathBuf::from_str(p).is_ok() {
                                Ok(Validation::Valid)
                            } else {
                                Ok(Validation::Invalid("Path is not valid".into()))
                            }
                        })
                        .prompt(session)?
                        .as_str()
                    {
                        "" => None,
                        s => {
                            Some(PathBuf::from_str(s).expect("expected to receive a valid string"))
                        }
                    },
                    gidnumber: match Text::new("Enter gid number:").prompt(session)?.as_str() {
                        "" => None,
                        s => Some(s.to_owned()),
                    },
                    displayname: match Text::new("Enter display name:").prompt(session)?.as_str() {
                        "" => None,
                        s => Some(s.to_owned()),
                    },
                    loginshell: match Text::new("Enter login shell:")
                        .with_validator(|p: &str| {
                            if p.eq("") | PathBuf::from_str(p).is_ok() {
                                Ok(Validation::Valid)
                            } else {
                                Ok(Validation::Invalid("Path is not valid".into()))
                            }
                        })
                        .prompt(session)?
                        .as_str()
                    {
                        "" => None,
                        s => {
                            Some(PathBuf::from_str(s).expect("expected to receive a valid string"))
                        }
                    },
                    mail: Text::new("Enter email:").prompt(session)?,
                    userpassword: match Password::new("Enter password:").prompt(session)?.as_str() {
                        "" => None,
                        s => Some(s.to_owned()),
                    },
                    random: match Select::new("Random password?:", vec!["true", "false"])
                        .prompt(session)?
                    {
                        "false" => None,
                        "true" => Some(true),
                        _ => None,
                    },
                    uidnumber: match Text::new("Enter uid number:").prompt(session)?.as_str() {
                        "" => None,
                        s => Some(s.to_owned()),
                    },
                    ou: Text::new("Enter organization:")
                        .prompt(session)?
                        .as_str()
                        .to_owned(),
                    title: match Text::new("Enter title:").prompt(session)?.as_str() {
                        "" => None,
                        s => Some(s.to_owned()),
                    },
                    ipasshpubkey: match Text::new("Enter an ssh key:")
                        .prompt(session)?
                        .as_str()
                        .split(",")
                        .into_iter()
                        .map(|s| s.to_owned())
                        .collect::<Vec<String>>()
                    {
                        s if s.len() > 0 => Some(s),
                        _ => None,
                    },
                    ipauserauthtype: match Text::new("Enter user auth type:")
                        .with_help_message(
                            "Possibly: none, password, radius, otp, pkinit, hardened, idp",
                        )
                        .prompt(session)?
                        .as_str()
                    {
                        "none" => None,
                        "password" => Some("password".to_owned()),
                        "radius" => Some("radius".to_owned()),
                        "otp" => Some("otp".to_owned()),
                        "pkinit" => Some("pkinit".to_owned()),
                        "hardened" => Some("hardened".to_owned()),
                        "idp" => Some("idp".to_owned()),
                        _ => None,
                    },
                    userclass: match Text::new("Enter user class:").prompt(session)?.as_str() {
                        "" => None,
                        s => Some(s.to_owned()),
                    },
                    usercertificate: match Text::new("Enter user cert data:")
                        .prompt(session)?
                        .as_str()
                    {
                        "" => None,
                        s => Some(s.to_owned()),
                    },
                };
                let res = ipa_instance.create_user(new_user, false).await;
                match res {
                    Ok(user) => {
                        let _ = notifications::send_new_account_notification(
                            &notifications::Env {
                                project: "anuket".to_owned(), // IPA is project independent. Any valid project name works here.
                            },
                            &notifications::IPAInfo {
                                username: user.uid,
                                password: user.userpassword.unwrap(),
                            },
                        )
                        .await;
                    }
                    Err(e) => writeln!(session, "Failed to create user with error: {e}")?,
                }
            }
            "Get user" => {
                let username = Text::new("Enter uid:").prompt(session)?;

                let all =
                    match Select::new("Show all data?:", vec!["true", "false"]).prompt(session)? {
                        "false" => false,
                        "true" => true,
                        _ => false,
                    };

                let res = ipa_instance.find_matching_user(username, all, false).await;
                match res {
                    Ok(u) => writeln!(
                        session,
                        "{}",
                        serde_json::to_string_pretty(&u).expect("Expected to serialize")
                    )?,
                    Err(e) => writeln!(session, "Failed to find user with error: {e}")?,
                }
            }
            "Update user" => {
                let username = Text::new("Enter uid:").prompt(session)?;

                let mut new_data: HashMap<String, UserData> = HashMap::new();
                let mut add_data: HashMap<String, UserData> = HashMap::new();

                let options: Vec<UserData> = vec![
                    UserData::uid(None),
                    UserData::givenname(None),
                    UserData::sn(None),
                    UserData::cn(None),
                    UserData::displayname(None),
                    UserData::homedirectory(None),
                    UserData::loginshell(None),
                    UserData::mail(None),
                    UserData::userpassword(None),
                    UserData::uidnumber(None),
                    UserData::gidnumber(None),
                    UserData::ou(None),
                    UserData::ipasshpubkey(None),
                    UserData::ipauserauthtype(None),
                    UserData::userclass(None),
                    UserData::usercertificate(None),
                    UserData::rename(None),
                ];
                loop {
                    let mut options_vec = vec!["Finish edits".to_owned()];
                    let mut vec: Vec<String> =
                        options.clone().into_iter().map(|f| f.to_string()).collect();
                    options_vec.append(&mut vec);

                    let edit_vec = vec!["Add", "Edit", "Delete"];
                    let mut add = false;

                    let userdata: UserData = match Select::new(
                        "Select an attribute to add, edit or delete",
                        options_vec,
                    )
                    .prompt(session)?
                    .as_str()
                    {
                        "Finish edits" => {
                            break;
                        }
                        "uid" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::uid(None),
                            "Edit" => UserData::uid(Some(Text::new("Enter uid:").prompt(session)?)),
                            _ => {
                                continue;
                            }
                        },
                        "givenname" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::givenname(None),
                            "Edit" => UserData::givenname(Some(
                                Text::new("Enter first name:").prompt(session)?,
                            )),
                            "Add" => {
                                add = true;
                                UserData::givenname(Some(
                                    Text::new("Enter first name:").prompt(session)?,
                                ))
                            }
                            _ => {
                                continue;
                            }
                        },
                        "sn" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::sn(None),
                            "Edit" => {
                                UserData::sn(Some(Text::new("Enter last name:").prompt(session)?))
                            }
                            "Add" => {
                                add = true;
                                UserData::sn(Some(Text::new("Enter last name:").prompt(session)?))
                            }
                            _ => {
                                continue;
                            }
                        },
                        "cn" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::cn(None),
                            "Edit" => {
                                UserData::cn(Some(Text::new("Enter full name:").prompt(session)?))
                            }
                            "Add" => {
                                add = true;
                                UserData::cn(Some(Text::new("Enter full name:").prompt(session)?))
                            }
                            _ => {
                                continue;
                            }
                        },
                        "displayname" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::displayname(None),
                            "Edit" => UserData::displayname(Some(
                                Text::new("Enter display name:").prompt(session)?,
                            )),
                            "Add" => {
                                add = true;
                                UserData::displayname(Some(
                                    Text::new("Enter display name:").prompt(session)?,
                                ))
                            }
                            _ => {
                                continue;
                            }
                        },
                        "homedirectory" => {
                            match Select::new("Action:", edit_vec).prompt(session)? {
                                "Delete" => UserData::homedirectory(None),
                                "Edit" => UserData::homedirectory(Some(
                                    match Text::new("Enter home dir:")
                                        .with_validator(|p: &str| {
                                            if p.eq("") | PathBuf::from_str(p).is_ok() {
                                                Ok(Validation::Valid)
                                            } else {
                                                Ok(Validation::Invalid("Path is not valid".into()))
                                            }
                                        })
                                        .prompt(session)?
                                        .as_str()
                                    {
                                        s => PathBuf::from_str(s)
                                            .expect("expected to receive a valid string"),
                                    },
                                )),
                                "Add" => {
                                    add = true;
                                    UserData::homedirectory(Some(
                                        match Text::new("Enter home dir:")
                                            .with_validator(|p: &str| {
                                                if p.eq("") | PathBuf::from_str(p).is_ok() {
                                                    Ok(Validation::Valid)
                                                } else {
                                                    Ok(Validation::Invalid(
                                                        "Path is not valid".into(),
                                                    ))
                                                }
                                            })
                                            .prompt(session)?
                                            .as_str()
                                        {
                                            s => PathBuf::from_str(s)
                                                .expect("expected to receive a valid string"),
                                        },
                                    ))
                                }
                                _ => {
                                    continue;
                                }
                            }
                        }
                        "loginshell" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::loginshell(None),
                            "Edit" => UserData::loginshell(Some(
                                match Text::new("Enter login shell:")
                                    .with_validator(|p: &str| {
                                        if p.eq("") | PathBuf::from_str(p).is_ok() {
                                            Ok(Validation::Valid)
                                        } else {
                                            Ok(Validation::Invalid("Path is not valid".into()))
                                        }
                                    })
                                    .prompt(session)?
                                    .as_str()
                                {
                                    s => PathBuf::from_str(s)
                                        .expect("expected to receive a valid string"),
                                },
                            )),
                            "Add" => {
                                add = true;
                                UserData::loginshell(Some(
                                    match Text::new("Enter login shell:")
                                        .with_validator(|p: &str| {
                                            if p.eq("") | PathBuf::from_str(p).is_ok() {
                                                Ok(Validation::Valid)
                                            } else {
                                                Ok(Validation::Invalid("Path is not valid".into()))
                                            }
                                        })
                                        .prompt(session)?
                                        .as_str()
                                    {
                                        s => PathBuf::from_str(s)
                                            .expect("expected to receive a valid string"),
                                    },
                                ))
                            }
                            _ => {
                                continue;
                            }
                        },
                        "mail" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::mail(None),
                            "Edit" => UserData::mail(Some(
                                Text::new("Enter first name:").prompt(session)?,
                            )),
                            "Add" => {
                                add = true;
                                UserData::mail(Some(
                                    Text::new("Enter first name:").prompt(session)?,
                                ))
                            }
                            _ => {
                                continue;
                            }
                        },
                        "userpassword" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::userpassword(None),
                            "Edit" => UserData::userpassword(
                                match Password::new("Enter :").prompt(session)?.as_str() {
                                    "" => None,
                                    s => Some(s.to_owned()),
                                },
                            ),
                            "Add" => {
                                add = true;
                                UserData::userpassword(
                                    match Password::new("Enter :").prompt(session)?.as_str() {
                                        "" => None,
                                        s => Some(s.to_owned()),
                                    },
                                )
                            }
                            _ => {
                                continue;
                            }
                        },
                        "uidnumber" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::uidnumber(None),
                            "Edit" => UserData::uidnumber(
                                match Text::new("Enter uid number:")
                                    .prompt(session)?
                                    .as_str()
                                    .parse::<i32>()
                                {
                                    Err(_) => None,
                                    Ok(i) => Some(i),
                                },
                            ),
                            "Add" => {
                                add = true;
                                UserData::uidnumber(
                                    match Text::new("Enter uid number:")
                                        .prompt(session)?
                                        .as_str()
                                        .parse::<i32>()
                                    {
                                        Err(_) => None,
                                        Ok(i) => Some(i),
                                    },
                                )
                            }
                            _ => {
                                continue;
                            }
                        },
                        "gidnumber" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::gidnumber(None),
                            "Edit" => UserData::gidnumber(
                                match Text::new("Enter gid number:")
                                    .prompt(session)?
                                    .as_str()
                                    .parse::<i32>()
                                {
                                    Err(_) => None,
                                    Ok(i) => Some(i),
                                },
                            ),
                            "Add" => {
                                add = true;
                                UserData::gidnumber(
                                    match Text::new("Enter gid number:")
                                        .prompt(session)?
                                        .as_str()
                                        .parse::<i32>()
                                    {
                                        Err(_) => None,
                                        Ok(i) => Some(i),
                                    },
                                )
                            }
                            _ => {
                                continue;
                            }
                        },
                        "ou" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::ou(None),
                            "Edit" => UserData::ou(
                                match Text::new("Enter organization:").prompt(session)?.as_str() {
                                    "" => None,
                                    s => Some(s.to_owned()),
                                },
                            ),
                            "Add" => {
                                add = true;
                                UserData::ou(
                                    match Text::new("Enter organization:").prompt(session)?.as_str()
                                    {
                                        "" => None,
                                        s => Some(s.to_owned()),
                                    },
                                )
                            }
                            _ => {
                                continue;
                            }
                        },
                        "ipasshpubkey" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::ipasshpubkey(None),
                            "Edit" => UserData::ipasshpubkey(
                                match Text::new("Enter comma separated ssh keys:")
                                    .prompt(session)?
                                    .as_str()
                                    .split(",")
                                    .into_iter()
                                    .map(|s| s.to_owned())
                                    .collect::<String>()
                                {
                                    s => Some(s),
                                },
                            ),
                            "Add" => {
                                add = true;
                                UserData::ipasshpubkey(
                                    match Text::new("Enter comma separated ssh keys:")
                                        .prompt(session)?
                                        .as_str()
                                        .split(",")
                                        .into_iter()
                                        .map(|s| s.to_owned())
                                        .collect::<String>()
                                    {
                                        s => Some(s),
                                    },
                                )
                            }
                            _ => {
                                continue;
                            }
                        },
                        "ipauserauthtype" => match Select::new("Action:", edit_vec)
                            .prompt(session)?
                        {
                            "Delete" => UserData::ipauserauthtype(None),
                            "Edit" => UserData::ipauserauthtype(
                                match Text::new("Enter user auth type:")
                                    .with_help_message(
                                        "Possibly: password, radius, otp, pkinit, hardened, idp",
                                    )
                                    .prompt(session)?
                                    .as_str()
                                {
                                    "" => None,
                                    "password" => Some("password".to_owned()),
                                    "radius" => Some("radius".to_owned()),
                                    "otp" => Some("otp".to_owned()),
                                    "pkinit" => Some("pkinit".to_owned()),
                                    "hardened" => Some("hardened".to_owned()),
                                    "idp" => Some("idp".to_owned()),
                                    _ => None,
                                },
                            ),
                            "Add" => {
                                add = true;
                                UserData::ipauserauthtype(
                                match Text::new("Enter user auth type:").with_help_message("Possibly: password, radius, otp, pkinit, hardened, idp").prompt(session)?.as_str() {
                                    "" => {None},
                                    "password" => {Some("password".to_owned())},
                                    "radius" => {Some("radius".to_owned())},
                                    "otp" => {Some("otp".to_owned())},
                                    "pkinit" => {Some("pkinit".to_owned())},
                                    "hardened" => {Some("hardened".to_owned())},
                                    "idp" => {Some("idp".to_owned())},
                                    _ => {None}}
                            )
                            }
                            _ => {
                                continue;
                            }
                        },
                        "userclass" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::userclass(None),
                            "Edit" => UserData::userclass(
                                match Text::new("Enter title:").prompt(session)?.as_str() {
                                    "" => None,
                                    s => Some(s.to_owned()),
                                },
                            ),
                            "Add" => {
                                add = true;
                                UserData::userclass(
                                    match Text::new("Enter title:").prompt(session)?.as_str() {
                                        "" => None,
                                        s => Some(s.to_owned()),
                                    },
                                )
                            }
                            _ => {
                                continue;
                            }
                        },
                        "usercertificate" => match Select::new("Action:", edit_vec)
                            .prompt(session)?
                        {
                            "Delete" => UserData::usercertificate(None),
                            "Edit" => UserData::usercertificate(
                                match Text::new("Enter user cert data:").prompt(session)?.as_str() {
                                    "" => None,
                                    s => Some(s.to_owned()),
                                },
                            ),
                            "Add" => {
                                add = true;
                                UserData::usercertificate(
                                    match Text::new("Enter user cert data:")
                                        .prompt(session)?
                                        .as_str()
                                    {
                                        "" => None,
                                        s => Some(s.to_owned()),
                                    },
                                )
                            }
                            _ => {
                                continue;
                            }
                        },
                        "rename" => match Select::new("Action:", edit_vec).prompt(session)? {
                            "Delete" => UserData::rename(None),
                            "Edit" => UserData::rename(Some(
                                Text::new("Enter new username:").prompt(session)?,
                            )),
                            "Add" => {
                                add = true;
                                UserData::rename(Some(
                                    Text::new("Enter new username:").prompt(session)?,
                                ))
                            }
                            _ => {
                                continue;
                            }
                        },
                        _ => {
                            continue;
                        }
                    };
                    if !add {
                        new_data.insert(userdata.to_string(), userdata);
                    } else {
                        add_data.insert(userdata.to_string(), userdata);
                    }
                }

                let res = ipa_instance
                    .update_user(
                        username,
                        add_data.into_values().into_iter().collect(),
                        new_data.into_values().into_iter().collect(),
                        false,
                    )
                    .await;
                match res {
                    Ok(u) => writeln!(
                        session,
                        "{}",
                        serde_json::to_string_pretty(&u).expect("Expected to serialize")
                    )?,
                    Err(e) => writeln!(session, "Failed to modify user with error: {e}")?,
                }
            }
            "Add user to group" => {
                let groupname = Text::new("Enter group name:").prompt(session)?;

                let username = Text::new("Enter uid:").prompt(session)?;

                let res = ipa_instance.group_add_user(groupname, username).await;
                match res {
                    Ok(u) => writeln!(
                        session,
                        "{}",
                        serde_json::to_string_pretty(&u).expect("Expected to serialize")
                    )?,
                    Err(e) => writeln!(session, "Failed to add user with error: {e}")?,
                }
            }
            "Remove user from group" => {
                let groupname = Text::new("Enter group name:").prompt(session)?;

                let username = Text::new("Enter uid:").prompt(session)?;

                let res = ipa_instance.group_remove_user(groupname, username).await;
                match res {
                    Ok(u) => writeln!(
                        session,
                        "{}",
                        serde_json::to_string_pretty(&u).expect("Expected to serialize")
                    )?,
                    Err(e) => writeln!(session, "Failed to remove user with error: {e}")?,
                }
            }
            &_ => {}
        }
    }
}

async fn create_booking(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let origin = Select::new(
        "select originating project:",
        settings()
            .projects
            .keys()
            .cloned()
            .into_iter()
            .collect_vec(),
    )
    .prompt(session)?;

    let blob = api::BookingBlob {
        origin,
        template_id: {
            let name = Select::new("Template:", get_templates(session, &mut transaction).await)
                .with_help_message("Select a template")
                .prompt(session)?;
            //Template::get(&mut transaction, name.data)
            name.data
            //Template::get_by_name(&mut transaction, name.).unwrap()[0].id
        },
        allowed_users: Text::new("Comma separated users")
            .prompt(session)?
            .as_str()
            .split(",")
            .into_iter()
            .map(|s| s.to_owned())
            .collect::<Vec<String>>(),
        global_cifile: Text::new("Ci-file:")
            .with_help_message("Enter a cifile")
            .prompt(session)?,
        metadata: BookingMetadataBlob {
            booking_id: Some(Text::new("Dashboard id:").prompt(session)?),
            owner: Some(Text::new("Owner:").prompt(session)?),
            lab: Some(Text::new("Lab:").prompt(session)?),
            purpose: Some(Text::new("Purpose:").prompt(session)?),
            project: Some(Text::new("Project:").prompt(session)?),
            length: Some(u64::from_str(
                Text::new("Sec:")
                    .with_validator(|input: &str| match u64::from_str(input) {
                        Ok(i) => match i {
                            _ => Ok(Validation::Valid),
                        },
                        Err(_) => Ok(Validation::Invalid("Input is not an integer".into())),
                    })
                    .prompt(session)?
                    .as_str(),
            )?),
        },
    };

    // insert booking blob into whatever db for the extra data

    writeln!(session, "Creating booking!")?;
    match make_aggregate(blob).await {
        Ok(agg) => {
            writeln!(session, "Aggregate id is: {:?}", agg.into_id().to_string())?;
            std::thread::sleep(Duration::from_secs(60));
        }
        Err(e) => writeln!(session, "Error creating booking: {}", e.to_string())?,
    }

    Ok(())
}

async fn import(mut session: &Server) -> Result<(), anyhow::Error> {
    loop {
        let mut client = new_client().await.expect("Expected to connect to db");
        let mut transaction = client
            .easy_transaction()
            .await
            .expect("Transaction creation error");

        let mut imp_vec = get_import_types();
        imp_vec.push("Select new task");
        let selected_col: Result<&str, _> = Select::new("Resource type:", imp_vec)
            .with_help_message("Select a type to import")
            .prompt(session);

        match selected_col.expect("Expected import type array to be non-empty") {
            "Select new task" => {
                break;
            }
            "Bookings" => {
                import_bookings(
                    session,
                    PathBuf::from("./config_data/laas-hosts/tascii/booking_dump.json"),
                )
                .await;
            }
            "Hosts" => {
                /*let conf_path =
                PathBuf::from("./config_data/laas-hosts/tascii/host_confluence.json");*/
                let dir = PathBuf::from("./config_data/laas-hosts/inventory");
                let mut proj_vec: Vec<String> = dir
                    .read_dir()
                    .expect("Expected to read import dir")
                    .filter_map(|h| {
                        if h.as_ref().expect("Expected host to exist").path().is_dir() {
                            Some(h.unwrap().path().to_str().unwrap().to_owned())
                        } else {
                            None
                        }
                    })
                    .collect();
                proj_vec.insert(0, "Select new task".to_owned());
                proj_vec.insert(1, "Import all".to_owned());
                let selected_host: Result<String, _> = Select::new("Choose host:", proj_vec)
                    .with_help_message("Select a type to import")
                    .prompt(session);
                match selected_host
                    .expect("Expected import host list to be non-empty")
                    .as_str()
                {
                    "Select new task" => {
                        break;
                    }
                    "Import all" => {
                        import_hosts(session).await;
                        writeln!(session, "Finished importing hosts")?;
                    }
                    proj => {
                        writeln!(session, "Importing {:?}", proj)?;

                        import_proj(
                            session,
                            &mut transaction,
                            PathBuf::from_str(proj).expect("Expected project to exist"),
                        )
                        .await;
                    }
                }
            }
            "Switches" => {
                //import_switches().await;
                let switch_path: PathBuf =
                    PathBuf::from_str("./config_data/laas-hosts/tascii/switches.json")
                        .expect("Expected to process into a PathBuf");

                import_switches(session, switch_path).await?;
            }
            "Vlans" => {
                let import_path = PathBuf::from_str("./config_data/laas-hosts/tascii/vlans.json")?;
                import_vlans_once(session, import_path)
                    .await
                    .expect("couldn't import vlans");
            }
            &_ => {}
        }
        transaction.commit().await.expect("couldn't commit import");
    }
    Ok(())
}

async fn expire_booking(_session: &Server) {
    todo!()
}

async fn extend_booking(_session: &Server) {
    todo!()
}

async fn get_usage_data(_session: &Server) {
    todo!()
}

async fn regenerate_ci_files(_session: &Server) {
    // Update booking in database with a freshly generated set of ci-files
    todo!()
}

/// Tasks that can be done via the cli are listed here

fn get_tasks(_session: &Server) -> Vec<&'static str> {
    vec![
        // Get useful info
        "Get Usage Data",
        // General interactions
        "Use database",
        "Use IPA",
        // Booking functions
        "Create booking",
        "Expire booking",
        "Extend booking", // will need to poke dashboard
        "Update Booking",
        "Regenerate Booking C-I files",
        // Use to import various things, takes place of add_server
        "Import",
        "Run Migrations",
        "Restart CLI",
        "Test LibLaaS",
        "Query",
        "Overrides",
        "Rerun Cleanup",
        "Rerun Deploy",
        "Manage Templates",
        "Recovery",
        "Shut Down Tascii",
        "Exit CLI",
    ]
}

/// Gets collections from the database to allow for interacting with them

fn get_collections(_session: &Server) -> Vec<String> {
    todo!();
}

/// List of crud operations for interacting with database entries
fn get_db_interactions() -> Vec<&'static str> {
    vec![
        "Create", // create by text entry
        "Edit", "List", "Delete",
    ]
}

/// List of things that can be imported
fn get_import_types() -> Vec<&'static str> {
    vec!["Hosts", "Switches", "Vlans", "Bookings"]
}

fn get_ipa_interactions() -> Vec<&'static str> {
    vec![
        "Create user",
        "Get user",
        "Update user",
        "Add user to group",
        "Remove user from group",
    ]
}

#[derive(Clone)]
struct SelectOption<T> {
    pub display: String,
    pub data: T,
}

impl<T> std::fmt::Display for SelectOption<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.display.fmt(f)
    }
}

async fn get_templates(
    _session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> Vec<SelectOption<FKey<Template>>> {
    Template::get_all(transaction)
        .await
        .unwrap()
        .into_iter()
        .map(|t| SelectOption {
            display: format!("{} owned by {:?} ({})", t.name, t.owner, t.description),
            data: t.id,
        })
        .collect()
}
