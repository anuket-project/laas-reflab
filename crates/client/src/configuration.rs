use dal::{AsEasyTransaction, FKey, get_db_pool, new_client};

use models::{
    dashboard::Image,
    inventory::{Flavor, FlavorCommands},
};
use std::io::Write;
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};

use crate::{
    areyousure, confirm,
    remote::{Select, Server, Text},
    select_flavor, select_image, select_template,
};

#[derive(Clone, Debug, Display, EnumString, EnumIter)]
enum ConfigureProvisioningOptions {
    #[strum(serialize = "Manage User Templates")]
    ManageUserTemplates,
    #[strum(serialize = "Manage Flavor / Image Run Command Overrides")]
    ManageFlavorCommands,
}

pub(crate) async fn submenu(session: &Server) -> Result<(), common::prelude::anyhow::Error> {
    match Select::new(
        "Select an option:",
        ConfigureProvisioningOptions::iter().collect(),
    )
    .prompt(session)?
    {
        ConfigureProvisioningOptions::ManageUserTemplates => Ok(modify_templates(session).await),
        ConfigureProvisioningOptions::ManageFlavorCommands => {
            Ok(manage_flavor_image_commands(session).await)
        }
    }
}

#[derive(Clone, Debug, Display, EnumString, EnumIter)]
enum ManageFlavorImageCommandChoice {
    Set,
    Delete,
    Cancel,
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
