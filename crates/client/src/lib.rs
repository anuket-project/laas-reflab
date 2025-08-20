// Copyright (c) 2023 University of New Hampshire
// SPDX-License-Identifier: MIT

#![doc = include_str!("../README.md")]
#![feature(
    result_flattening,
    let_chains,
    panic_backtrace_config,
    update_panic_count,
    panic_can_unwind
)]

pub mod mgmt_workflows;
pub mod remote;

mod ipa;
mod overrides;
mod queries;

use common::prelude::{
    anyhow, config::settings, inquire::validator::Validation, itertools::Itertools,
};
use dal::{new_client, AsEasyTransaction, DBTable, EasyTransaction, FKey, ID};
use liblaas::{
    booking::make_aggregate,
    web::api::{self, BookingMetadataBlob},
};
use mgmt_workflows::BootBookedHosts;

use models::{
    dashboard::{Aggregate, Instance, LifeCycleState, Template},
    inventory::{Host, Lab},
};
use remote::{Select, Server, Text};
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::{fmt::Formatter, str::FromStr, time::Duration};
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};
use tascii::prelude::Runtime;
use workflows::entry::DISPATCH;

/// Runs the cli
#[derive(Debug, Copy, Clone)]
pub enum LiblaasStateInstruction {
    ShutDown,
    DoNothing,
    ExitCLI,
}

#[derive(Clone, Debug, Display, EnumString, EnumIter)]
pub enum Command {
    #[strum(serialize = "Usage Data")]
    UsageData,
    #[strum(serialize = "IPA Utilities")]
    IPA,
    #[strum(serialize = "Create a Booking")]
    CreateBooking,
    #[strum(serialize = "Expire a Booking")]
    ExpireBooking,
    #[strum(serialize = "Extend a Booking")]
    ExtendBooking,
    #[strum(serialize = "Update a Boooking")]
    UpdateBooking,
    #[strum(serialize = "Regenerate CI Files")]
    BookingCI,
    #[strum(serialize = "Run Migrations")]
    Migrations,
    #[strum(serialize = "Restart CLI")]
    Restart,
    #[strum(serialize = "Run Tests")]
    Test,
    Query,
    Overrides,
    #[strum(serialize = "Manual Host Cleanup")]
    ManualCleanup,
    #[strum(serialize = "Manual Host Deploy")]
    ManualDeploy,
    #[strum(serialize = "Manage Email Templates")]
    ManageTemplates,
    #[strum(serialize = "Recovery")]
    Recovery,
    #[strum(serialize = "Shutdown Tascii")]
    Shutdown,
    #[strum(serialize = "Exit CLI")]
    Exit,
}

pub async fn cli_entry(
    tascii_rt: &'static Runtime,
    mut session: &Server,
) -> Result<LiblaasStateInstruction, anyhow::Error> {
    // Loop cli so users can do multiple things
    loop {
        let task =
            Select::new("What would you like to do?", Command::iter().collect()).prompt(session)?;

        match task {
            Command::Recovery => {
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
            Command::IPA => {
                ipa::use_ipa(session)
                    .await
                    .expect("couldn't finish use ipa");
            }
            // Booking functions
            Command::CreateBooking => create_booking(session)
                .await
                .expect("couldn't create booking"), // Dispatches booking creation
            Command::ExpireBooking => expire_booking(session).await, // Dispatches the cleanup task
            Command::ExtendBooking => extend_booking(session).await, // Will need to poke dashboard
            Command::BookingCI => regenerate_ci_files(session).await,
            Command::ManualCleanup => {
                let id = Text::new("Enter UUID for cleanup task to rerun: ").prompt(session)?;

                areyousure(session)?;

                let uid = FKey::from_id(ID::from_str(&id).unwrap());
                DISPATCH
                    .get()
                    .unwrap()
                    .send(workflows::entry::Action::CleanupBooking { agg_id: uid })?;
                let _ = writeln!(session, "Successfully started cleanup");
            }

            Command::ManageTemplates => modify_templates(session).await,

            Command::ManualDeploy => {
                let id = Text::new("Enter UUID for aggregate to rerun: ").prompt(session)?;

                areyousure(session)?;

                let uid = FKey::from_id(ID::from_str(&id).unwrap());
                DISPATCH
                    .get()
                    .unwrap()
                    .send(workflows::entry::Action::DeployBooking { agg_id: uid })?;
                let _ = writeln!(session, "Successfully started deploy");
            }

            Command::Overrides => overrides::overrides(session, tascii_rt).await?,

            Command::Query => queries::query(session).await.unwrap(),

            // Get useful info
            Command::UsageData => {
                get_usage_data(session).await;
            }
            Command::Migrations => {
                dal::initialize().await.unwrap();
            }
            Command::Restart => return Ok(LiblaasStateInstruction::DoNothing),
            Command::Shutdown => {
                areyousure(session)?;
                return Ok(LiblaasStateInstruction::ShutDown);
            }
            Command::Exit => return Ok(LiblaasStateInstruction::ExitCLI),
            _ => todo!(),
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

#[derive(Debug, Clone, EnumIter, EnumString, Display)]
pub enum YesNo {
    No,
    Yes,
}

fn areyousure(session: &Server) -> Result<(), anyhow::Error> {
    match Select::new("Are you sure?:", YesNo::iter().collect())
        .prompt(session)
        .unwrap()
    {
        YesNo::No => Err(anyhow::Error::msg("user was not sure")),
        YesNo::Yes => Ok(()),
    }
}

async fn get_lab(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Lab>, anyhow::Error> {
    match Lab::select().run(transaction).await {
        Ok(lab_list) => {
            let name = Select::new(
                "Select a Lab: ",
                lab_list.iter().map(|lab| lab.name.clone()).collect_vec(),
            )
            .prompt(session)
            .unwrap();
            match Lab::get_by_name(transaction, name).await {
                Ok(opt_lab) => match opt_lab {
                    Some(l) => Ok(l.id),
                    None => Err(anyhow::Error::msg("Error Lab does not exist".to_string())),
                },
                Err(e) => Err(anyhow::Error::msg(format!("Error finding lab: {}", e))),
            }
        }
        Err(e) => Err(anyhow::Error::msg(format!(
            "Failed to retrieve lab list: {e}"
        ))),
    }
}

impl std::fmt::Display for DispHost {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let host = self.host.clone();
        write!(f, "{}", host.server_name)
    }
}

async fn select_host(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Host>, anyhow::Error> {
    let hosts = Host::select().run(transaction).await?;

    let mut disps = Vec::new();

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

    Ok(Select::new("select an aggregate:", disps)
        .prompt(session)?
        .id)
}

impl std::fmt::Display for DispAgg {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut instances = String::new();

        for inst in &self.hosts {
            writeln!(instances, " - {}", inst)?;
        }

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

async fn select_template(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Template>, anyhow::Error> {
    let temps = Template::select().run(transaction).await?;

    let mut disps = Vec::new();

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

        DispInst {
            id: inst.id,
            hostname,
            host,
        }
    }
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

    Ok(Select::new("select an instance:", disps)
        .prompt(session)?
        .id)
}

async fn create_booking(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let origin = Select::new(
        "select originating project:",
        settings().projects.keys().cloned().collect_vec(),
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
            .split(',')
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
            details: Some(Text::new("Details").prompt(session)?),
            length: Some(u64::from_str(
                Text::new("Sec:")
                    .with_validator(|input: &str| match u64::from_str(input) {
                        Ok(_) => Ok(Validation::Valid),
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
        Err(e) => writeln!(session, "Error creating booking: {}", e)?,
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
