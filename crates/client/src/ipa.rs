use crate::remote::{Password, Select, Server, Text};
use common::prelude::{inquire::validator::Validation, serde_json};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};
use users::ipa::{UserData, *};
use workflows::resource_management::vpn::single_vpn_sync_for_user;

#[derive(Clone, Debug, Display, EnumString, EnumIter)]
pub enum IpaCommand {
    #[strum(serialize = "Create a User")]
    Create,
    #[strum(serialize = "Retrieve a User")]
    Get,
    #[strum(serialize = "Update a User")]
    Update,
    #[strum(serialize = "Add a User to a Group")]
    AddGroup,
    #[strum(serialize = "Remove a User from a Group")]
    RemoveGroup,
    #[strum(serialize = "List Groups for a User")]
    ListGroups,
    #[strum(serialize = "Sync VPN Configuration for a User")]
    SyncVpn,
}

pub async fn use_ipa(session: &Server) -> Result<(), common::prelude::anyhow::Error> {
    let mut ipa_instance = IPA::init()
        .await
        .expect("Expected to initialize IPA instance");

    loop {
        match Select::new("Select an IPA interaction:", IpaCommand::iter().collect())
            .prompt(session)?
        {
            IpaCommand::Create => create_user(&mut ipa_instance, session).await?,
            IpaCommand::Get => get_user(&mut ipa_instance, session).await?,
            IpaCommand::Update => update_user(&mut ipa_instance, session).await?,
            IpaCommand::AddGroup => modify_group(&mut ipa_instance, session, true).await?,
            IpaCommand::RemoveGroup => modify_group(&mut ipa_instance, session, false).await?,
            IpaCommand::ListGroups => list_groups(&mut ipa_instance, session).await?,
            IpaCommand::SyncVpn => sync_vpn(session).await?,
        }
    }
}

async fn create_user(
    ipa_instance: &mut IPA,
    mut session: &Server,
) -> Result<(), common::prelude::anyhow::Error> {
    let new_user = User {
        uid: prompt_text(session, "Enter uid:")?,
        givenname: prompt_text(session, "Enter first name:")?,
        sn: prompt_text(session, "Enter last name:")?,
        cn: prompt_optional_text(session, "Enter full name:")?,
        homedirectory: prompt_optional_path(session, "Enter home dir:")?,
        gidnumber: prompt_optional_text(session, "Enter gid number:")?,
        displayname: prompt_optional_text(session, "Enter display name:")?,
        loginshell: prompt_optional_path(session, "Enter login shell:")?,
        mail: prompt_text(session, "Enter email:")?,
        userpassword: prompt_optional_password(session, "Enter password:")?,
        random: prompt_boolean(session, "Random password?:")?,
        uidnumber: prompt_optional_text(session, "Enter uid number:")?,
        ou: prompt_text(session, "Enter organization:")?,
        title: prompt_optional_text(session, "Enter title:")?,
        ipasshpubkey: prompt_optional_comma_separated(session, "Enter ssh keys:")?,
        ipauserauthtype: prompt_user_auth_type(session)?,
        userclass: prompt_optional_text(session, "Enter user class:")?,
        usercertificate: prompt_optional_text(session, "Enter user cert data:")?,
    };

    match ipa_instance.create_user(new_user, false).await {
        Ok(user) => {
            notifications::send_new_account_notification(
                &notifications::Env {
                    project: "anuket".to_owned(),
                },
                &notifications::IPAInfo {
                    username: user.uid,
                    password: user.userpassword.unwrap(),
                },
            )
            .await
            .unwrap();
        }
        Err(e) => writeln!(session, "Failed to create user with error: {e}")?,
    }

    Ok(())
}

async fn get_user(
    ipa_instance: &mut IPA,
    mut session: &Server,
) -> Result<(), common::prelude::anyhow::Error> {
    let username = prompt_text(session, "Enter uid:")?;
    let all = prompt_boolean(session, "Show all data?:")?;

    match ipa_instance
        .find_matching_user(username, all.expect("Expected All"), false)
        .await
    {
        Ok(user) => writeln!(
            session,
            "{}",
            serde_json::to_string_pretty(&user).expect("Expected to serialize")
        )?,
        Err(e) => writeln!(session, "Failed to find user with error: {e}")?,
    }

    Ok(())
}

async fn update_user(
    ipa_instance: &mut IPA,
    mut session: &Server,
) -> Result<(), common::prelude::anyhow::Error> {
    let username = prompt_text(session, "Enter uid:")?;
    let mut new_data: HashMap<String, UserData> = HashMap::new();
    let mut add_data: HashMap<String, UserData> = HashMap::new();

    loop {
        let action = Select::new(
            "Select an attribute to add, edit, or delete",
            UserData::iter().collect::<Vec<_>>(),
        )
        .prompt(session)?;

        if action.to_string() == "Finish edits" {
            break;
        }

        let edit_action = Select::new("Action:", EditAction::iter().collect()).prompt(session)?;

        let mut add = false;
        let userdata = match action {
            UserData::uid(_) => {
                handle_user_data(session, &edit_action, |value| UserData::uid(Some(value)))
            }
            UserData::givenname(_) => handle_user_data(session, &edit_action, |value| {
                UserData::givenname(Some(value))
            }),
            UserData::sn(_) => {
                handle_user_data(session, &edit_action, |value| UserData::sn(Some(value)))
            }
            UserData::cn(_) => {
                handle_user_data(session, &edit_action, |value| UserData::cn(Some(value)))
            }
            UserData::displayname(_) => handle_user_data(session, &edit_action, |value| {
                UserData::displayname(Some(value))
            }),
            UserData::homedirectory(_) => handle_user_data(session, &edit_action, |value| {
                UserData::homedirectory(Some(
                    PathBuf::from_str(&value).expect("expected to receive a valid string"),
                ))
            }),
            UserData::loginshell(_) => handle_user_data(session, &edit_action, |value| {
                UserData::loginshell(Some(
                    PathBuf::from_str(&value).expect("expected to receive a valid string"),
                ))
            }),
            UserData::mail(_) => {
                handle_user_data(session, &edit_action, |value| UserData::mail(Some(value)))
            }
            UserData::userpassword(_) => handle_user_data(session, &edit_action, |value| {
                UserData::userpassword(Some(value))
            }),
            UserData::uidnumber(_) => handle_user_data(session, &edit_action, |value| {
                UserData::uidnumber(Some(value.parse().expect("Expected valid integer")))
            }),
            UserData::gidnumber(_) => handle_user_data(session, &edit_action, |value| {
                UserData::gidnumber(Some(value.parse().expect("Expected valid integer")))
            }),
            UserData::ou(_) => {
                handle_user_data(session, &edit_action, |value| UserData::ou(Some(value)))
            }
            UserData::ipasshpubkey(_) => handle_user_data(session, &edit_action, |value| {
                UserData::ipasshpubkey(Some(value.split(',').map(|s| s.to_owned()).collect()))
            }),
            UserData::ipauserauthtype(_) => handle_user_data(session, &edit_action, |value| {
                UserData::ipauserauthtype(Some(value))
            }),
            UserData::userclass(_) => handle_user_data(session, &edit_action, |value| {
                UserData::userclass(Some(value))
            }),
            UserData::usercertificate(_) => handle_user_data(session, &edit_action, |value| {
                UserData::usercertificate(Some(value))
            }),
            UserData::rename(_) => {
                handle_user_data(session, &edit_action, |value| UserData::rename(Some(value)))
            }
        };

        if edit_action == EditAction::Add {
            add = true;
        }

        if !add {
            new_data.insert(action.to_string(), userdata);
        } else {
            add_data.insert(action.to_string(), userdata);
        }
    }

    match ipa_instance
        .update_user(
            username,
            add_data.into_values().collect(),
            new_data.into_values().collect(),
            false,
        )
        .await
    {
        Ok(user) => writeln!(
            session,
            "{}",
            serde_json::to_string_pretty(&user).expect("Expected to serialize")
        )?,
        Err(e) => writeln!(session, "Failed to modify user with error: {e}")?,
    }

    Ok(())
}

async fn modify_group(
    ipa_instance: &mut IPA,
    mut session: &Server,
    add: bool,
) -> Result<(), common::prelude::anyhow::Error> {
    let groupname = prompt_text(session, "Enter group name:")?;
    let username = prompt_text(session, "Enter uid:")?;

    let res = if add {
        ipa_instance.group_add_user(&groupname, &username).await
    } else {
        ipa_instance.group_remove_user(&groupname, &username).await
    };

    match res {
        Ok(group) => writeln!(
            session,
            "{}",
            serde_json::to_string_pretty(&group).expect("Expected to serialize")
        )?,
        Err(e) => writeln!(session, "Failed to modify group with error: {e}")?,
    }

    Ok(())
}

async fn list_groups(
    ipa_instance: &mut IPA,
    mut session: &Server,
) -> Result<(), common::prelude::anyhow::Error> {
    let username = prompt_text(session, "Enter username:")?;
    match ipa_instance.group_find_user(&username).await {
        Ok(groups) => writeln!(session, "IPA groups for {username}: {groups:?}")?,
        Err(e) => writeln!(
            session,
            "Failed to get groups for user {username} with error: {e}"
        )?,
    }
    Ok(())
}

async fn sync_vpn(mut session: &Server) -> Result<(), common::prelude::anyhow::Error> {
    let username = prompt_text(session, "Enter username:")?;
    match single_vpn_sync_for_user(&username).await {
        Ok(results) => {
            writeln!(session, "Successfully updated VPN groups for {username}\nGroups added: {:?}\nGroups removed: {:?}", results.0, results.1)?;
        }
        Err(e) => writeln!(session, "Failed to sync vpn for {username}: {e}")?,
    }
    Ok(())
}

fn prompt_text(session: &Server, message: &str) -> Result<String, common::prelude::anyhow::Error> {
    Text::new(message).prompt(session)
}

fn prompt_optional_text(
    session: &Server,
    message: &str,
) -> Result<Option<String>, common::prelude::anyhow::Error> {
    match Text::new(message).prompt(session)?.as_str() {
        "" => Ok(None),
        s => Ok(Some(s.to_owned())),
    }
}

fn prompt_optional_path(
    session: &Server,
    message: &str,
) -> Result<Option<PathBuf>, common::prelude::anyhow::Error> {
    match Text::new(message)
        .with_validator(|p: &str| {
            if p.is_empty() || PathBuf::from_str(p).is_ok() {
                Ok(Validation::Valid)
            } else {
                Ok(Validation::Invalid("Path is not valid".into()))
            }
        })
        .prompt(session)?
        .as_str()
    {
        "" => Ok(None),
        s => Ok(Some(
            PathBuf::from_str(s).expect("expected to receive a valid string"),
        )),
    }
}

fn prompt_optional_password(
    session: &Server,
    message: &str,
) -> Result<Option<String>, common::prelude::anyhow::Error> {
    match Password::new(message).prompt(session)?.as_str() {
        "" => Ok(None),
        s => Ok(Some(s.to_owned())),
    }
}

fn prompt_boolean(
    session: &Server,
    message: &str,
) -> Result<Option<bool>, common::prelude::anyhow::Error> {
    match Select::new(message, vec!["true", "false"]).prompt(session)? {
        "false" => Ok(None),
        "true" => Ok(Some(true)),
        _ => Ok(None),
    }
}

fn prompt_optional_comma_separated(
    session: &Server,
    message: &str,
) -> Result<Option<Vec<String>>, common::prelude::anyhow::Error> {
    match Text::new(message).prompt(session)?.as_str() {
        "" => Ok(None),
        s => Ok(Some(s.split(',').map(|s| s.to_owned()).collect())),
    }
}

fn prompt_user_auth_type(
    session: &Server,
) -> Result<Option<String>, common::prelude::anyhow::Error> {
    match Text::new("Enter user auth type:")
        .with_help_message("Possibly: none, password, radius, otp, pkinit, hardened, idp")
        .prompt(session)?
        .as_str()
    {
        "none" => Ok(None),
        "password" => Ok(Some("password".to_owned())),
        "radius" => Ok(Some("radius".to_owned())),
        "otp" => Ok(Some("otp".to_owned())),
        "pkinit" => Ok(Some("pkinit".to_owned())),
        "hardened" => Ok(Some("hardened".to_owned())),
        "idp" => Ok(Some("idp".to_owned())),
        _ => Ok(None),
    }
}

fn handle_user_data<F>(session: &Server, edit_action: &EditAction, constructor: F) -> UserData
where
    F: FnOnce(String) -> UserData,
{
    match edit_action {
        EditAction::Delete => constructor(String::new()),
        EditAction::Edit => constructor(prompt_text(session, "Enter value:").unwrap()),
        EditAction::Add => constructor(prompt_text(session, "Enter value:").unwrap()),
    }
}

#[derive(Debug, Clone, EnumIter, Display, Eq, PartialEq)]
pub enum EditAction {
    Delete,
    Edit,
    Add,
}

