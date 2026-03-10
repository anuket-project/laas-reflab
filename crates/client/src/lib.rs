#![doc = include_str!("../README.md")]

pub mod mgmt_workflows;
pub mod remote;

mod ipa;
mod notifications;
mod overrides;
mod queries;
mod switch_test;
mod test_utils;

use common::prelude::{anyhow, itertools::Itertools};
use dal::{AsEasyTransaction, DBTable, EasyTransaction, FKey, ID, get_db_pool, new_client};
use mgmt_workflows::BootBookedHosts;

use models::{
    dashboard::{Aggregate, Image, Instance, LifeCycleState, Template},
    inventory::{Flavor, FlavorCommands, Host, Lab},
};
use remote::{Select, Server, Text};
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::{fmt::Formatter, str::FromStr};
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};
use tascii::prelude::Runtime;
use workflows::entry::{Action, DISPATCH};

/// Runs the cli
#[derive(Debug, Copy, Clone)]
pub enum LiblaasStateInstruction {
    Continue,
    Exit,
}

#[derive(Clone, Debug, Display, EnumString, EnumIter)]
pub enum Command {
    #[strum(serialize = "IPA Utilities")]
    IPA,
    Query,
    Overrides,
    Notifications,
    #[strum(serialize = "Manual Host Cleanup")]
    ManualCleanup,
    #[strum(serialize = "Manual Host Deploy")]
    ManualDeploy,
    #[strum(serialize = "Manage User Templates")]
    ManageTemplates,
    #[strum(serialize = "Recovery")]
    Recovery,
    #[strum(serialize = "Testing Utilities")]
    TestingUtils,
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
            Command::ManualCleanup => {
                let id = Text::new("Enter UUID for cleanup task to rerun: ").prompt(session)?;

                areyousure(session)?;

                let uid = FKey::from_id(ID::from_str(&id).unwrap());
                DISPATCH
                    .get()
                    .unwrap()
                    .send(Action::CleanupBooking { agg_id: uid })?;
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
                    .send(Action::DeployBooking { agg_id: uid })?;
                let _ = writeln!(session, "Successfully started deploy");
            }

            Command::Overrides => overrides::overrides(session, tascii_rt).await?,
            Command::Notifications => {
                notifications::notification_actions(session, tascii_rt).await?;
            }
            Command::Query => queries::query(session).await.unwrap(),
            Command::TestingUtils => {
                test_utils::test_utils(session).await?;
            }
            Command::Exit => return Ok(LiblaasStateInstruction::Exit),
        }
    }
}

async fn manage_flavor_image_commands(mut session: &Server) {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let flavor = select_flavor(session, &mut transaction).await.unwrap();

    let image = select_image(session, &mut transaction).await.unwrap();

    transaction.commit().await.unwrap();

    let pool = get_db_pool().await.unwrap();

    let existing_flavor_commands = FlavorCommands::get_for_flavor_image_ids(
        &flavor.into_id().into_uuid(),
        &image.into_id().into_uuid(),
        &pool,
    )
    .await
    .unwrap();

    let _ = writeln!(
        session,
        "Currently configured Flavor/Image Commands - {existing_flavor_commands:?}"
    );

    match Select::new(
        "Select an operation",
        ManageFlavorImageCommandChoice::iter().collect(),
    )
    .prompt(session)
    .unwrap()
    {
        ManageFlavorImageCommandChoice::Set => {
            set_flavor_image_command(session, flavor, image).await
        }
        ManageFlavorImageCommandChoice::Delete => {
            delete_flavor_image_command(session, flavor, image).await
        }
        ManageFlavorImageCommandChoice::Cancel => {}
    }
}

async fn set_flavor_image_command(mut session: &Server, flavor: FKey<Flavor>, image: FKey<Image>) {
    let mut selecting = true;

    let mut commands: Vec<String> = vec![];

    while selecting {
        commands.push(Text::new("Enter command:").prompt(session).unwrap());
        selecting = confirm(session, "Enter another?");
    }

    let _ = writeln!(session, "Entered Commands: {commands:?}");

    if let Err(_) = areyousure(session) {
        let _ = writeln!(session, "Operation cancelled.");
        return;
    }

    let pool = get_db_pool().await.unwrap();

    let res = FlavorCommands::set_for_flavor_image_ids(
        &flavor.into_id().into_uuid(),
        &image.into_id().into_uuid(),
        commands,
        &pool,
    )
    .await
    .unwrap();
    let _ = writeln!(session, "Successfully set Flavor / Image command {res:?}");
}

async fn delete_flavor_image_command(
    mut session: &Server,
    flavor: FKey<Flavor>,
    image: FKey<Image>,
) {
    if let Err(_) = areyousure(session) {
        let _ = writeln!(session, "Operation cancelled.");
        return;
    }

    let pool = get_db_pool().await.unwrap();

    FlavorCommands::delete_for_flavor_image_ids(
        &flavor.into_id().into_uuid(),
        &image.into_id().into_uuid(),
        &pool,
    )
    .await
    .unwrap();
    let _ = writeln!(session, "Successfully deleted Flavor / Image command.");
}

#[derive(Clone, Debug, Display, EnumString, EnumIter)]
enum ManageFlavorImageCommandChoice {
    Set,
    Delete,
    Cancel,
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
fn confirm(session: &Server, message: &str) -> bool {
    match Select::new(message, YesNo::iter().collect())
        .prompt(session)
        .unwrap()
    {
        YesNo::No => false,
        YesNo::Yes => true,
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

#[derive(Clone, Debug)]
struct DispFlavor {
    id: FKey<Flavor>,
    name: String,
}

#[derive(Clone, Debug)]
struct DispImage {
    id: FKey<Image>,
    name: String,
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

impl DispFlavor {
    pub fn from_flavor(flavor: Flavor) -> Self {
        DispFlavor {
            id: flavor.id,
            name: flavor.name,
        }
    }
}

impl std::fmt::Display for DispFlavor {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.id.into_id())
    }
}

impl DispImage {
    pub fn from_image(image: Image) -> Self {
        DispImage {
            id: image.id,
            name: image.name,
        }
    }
}

impl std::fmt::Display for DispImage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.id.into_id())
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

async fn select_flavor(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Flavor>, anyhow::Error> {
    let flavor_choices = Flavor::select()
        .run(transaction)
        .await?
        .into_iter()
        .map(|f| DispFlavor::from_flavor(f.into_inner()))
        .collect();

    Ok(Select::new("Select a flavor:", flavor_choices)
        .prompt(session)?
        .id)
}

async fn select_image(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Image>, anyhow::Error> {
    let image_choices = Image::select()
        .run(transaction)
        .await?
        .into_iter()
        .map(|f| DispImage::from_image(f.into_inner()))
        .collect();

    Ok(Select::new("Select an image:", image_choices)
        .prompt(session)?
        .id)
}
