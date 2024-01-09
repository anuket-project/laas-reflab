//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

#![allow(unused_attributes, unused_variables, dead_code, unused, unused_imports)]

use email::send;
use models::dashboard::AggregateConfiguration;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::File,
    io::{prelude::*, BufReader},
    path::{Path, PathBuf},
};
use tera::Tera;
pub mod email;

use common::prelude::{anyhow, chrono, futures, itertools::Itertools, serde_json::json, tracing};
use config::{settings, RenderTarget, Situation};

pub type Username = String;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub enum Method {
    Email(),
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct AttachmentInfo {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Notification {
    title: String,
    send_to: Username,
    by_methods: Vec<Method>,
    situation: Situation,
    project: String,
    /// For simple templating
    context: tera::Context,
    attachment: Option<AttachmentInfo>,
}

fn render(notification: &Notification, target: RenderTarget) -> Result<String, anyhow::Error> {
    tracing::debug!("Getting a template with target {target:?} for notification {notification:?}");
    let template_name =
        templates::retrieve(notification.project.clone(), notification.situation, target)
            .expect("no template found matching query");

    let rendered = TERA.render(&template_name, &notification.context)?;

    Ok(rendered)
}

fn preferred_methods(for_user: &Username) -> Vec<Method> {
    vec![Method::Email()]
}

#[derive(Debug)]
pub struct Env {
    pub project: String,
}

pub struct BookingInfo {
    pub owner: Username,
    pub collaborators: Vec<Username>,
    pub lab: String,
    pub id: String,
    pub template: String,
    pub purpose: String,
    pub start_date: Option<chrono::DateTime<chrono::Utc>>,
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
    pub dashboard_url: String,
    pub configuration: AggregateConfiguration,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IPAInfo {
    pub username: String,
    pub password: String,
}

pub fn read_styles(path: &str) -> Result<String, anyhow::Error> {
    let file = File::open(path)?;
    let mut buf_reader = BufReader::new(file);
    let mut contents = String::new();
    buf_reader.read_to_string(&mut contents)?;
    Ok(contents)
}

pub async fn send_booking_notification(
    env: &Env,
    info: &BookingInfo,
    situation: Situation,
    owner_title: String,
    collab_title: String,
) -> Result<(), Vec<anyhow::Error>> {
    let users = info
        .collaborators
        .clone()
        .into_iter()
        .map(|un| (false, un))
        .chain(vec![(true, info.owner.clone())])
        .collect_vec();
    let mut errors: Vec<anyhow::Error> = Vec::new();

    for (is_owner, username) in users {
        let start = match info.start_date {
            Some(s) => s.to_rfc2822(),
            None => "None".to_owned(),
        };
        let end = match info.end_date {
            Some(e) => e.to_rfc2822(),
            None => "None".to_owned(),
        };

        let styles = read_styles(
            settings()
                .projects
                .get(env.project.clone().as_str())
                .unwrap()
                .styles_path
                .as_str(),
        )
        .expect("Failed to read styles");

        let styles_json: serde_json::Value =
            serde_json::from_str(&styles).expect("Failed to parse JSON");

        let mut context = tera::Context::new();
        // insert styles into the context
        context.insert("styles", &styles_json);

        // add the rest of the context
        context.insert(
            "booking",
            &json!({
                "id": info.id,
                "lab": info.lab,
                "purpose": info.purpose,
                "template": info.template,
                "project": env.project.clone(),
                "owner": info.owner,
                "collaborators": info.collaborators,
                "start": start,
                "end": end,
                "ipmi_password": info.configuration.ipmi_password,
                "ipmi_username": info.configuration.ipmi_username,
            }),
        );
        context.insert("owner", &is_owner);
        context.insert("dashboard_url", &info.dashboard_url);

        // create te notification
        let notification = Notification {
            title: if is_owner {
                owner_title.clone()
            } else {
                collab_title.clone()
            },
            send_to: username.clone(),
            by_methods: preferred_methods(&username.clone()),
            situation,
            project: env.project.clone(),
            context, // Use the merged context here
            attachment: None,
        };

        match send(env, notification).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to send email to {username} with error {e:#?}");
                errors.push(e)
            }
        }
    }
    if (errors.is_empty()) {
        Ok(())
    } else {
        Err(errors)
    }
}

pub async fn send_test_email(
    status: Situation,
    project: String,
    owner_user: Username,
    collab_users: Option<Vec<Username>>,
) -> Result<(), Vec<anyhow::Error>> {
    // create dummy BookingInfo
    let dummy_info = BookingInfo {
        owner: owner_user,
        collaborators: collab_users.unwrap_or_default(),
        lab: "Some Lab".to_owned(),
        id: "12345".to_owned(),
        template: "Test Pod".to_owned(),
        purpose: "Email Testing".to_owned(),
        start_date: Some(chrono::Utc::now()),
        end_date: Some(chrono::Utc::now()),
        dashboard_url: "https://example.com".to_owned(),
        configuration: AggregateConfiguration {
            ipmi_username: "fedora_the_explorer".to_owned(),
            ipmi_password: "youwillneverguessthis".to_owned(),
        },
    };

    let dummy_env = Env { project };

    match status {
        Situation::BookingExpired => booking_ended(&dummy_env, &dummy_info).await,
        Situation::BookingCreated => booking_started(&dummy_env, &dummy_info).await,
        Situation::BookingExpiring => booking_ending(&dummy_env, &dummy_info).await,
        _ => Err(vec![anyhow::Error::msg(
            "Invalid status for test email. Must be BookingExpired, BookingCreated or BookingExpiring",
        )]),
    }
}

pub async fn booking_started(env: &Env, info: &BookingInfo) -> Result<(), Vec<anyhow::Error>> {
    send_booking_notification(
        env,
        info,
        Situation::BookingCreated,
        "You Have Created a New Booking.".to_owned(),
        "You Have Been Added To a New Booking.".to_owned(),
    )
    .await
}

pub async fn booking_ending(env: &Env, info: &BookingInfo) -> Result<(), Vec<anyhow::Error>> {
    send_booking_notification(
        env,
        info,
        Situation::BookingExpiring,
        "Your Booking Is About to Expire.".to_owned(),
        "A Booking You Collaborate On Is About to Expire.".to_owned(),
    )
    .await
}

pub async fn booking_ended(env: &Env, info: &BookingInfo) -> Result<(), Vec<anyhow::Error>> {
    send_booking_notification(
        env,
        info,
        Situation::BookingExpired,
        "Your Booking Has Expired.".to_owned(),
        "A Booking You Collaborate On Has Expired.".to_owned(),
    )
    .await
}

/// Send email containing ipa username, temp password, openvpn config, and instructions
pub async fn send_new_account_notification(
    env: &Env,
    info: &IPAInfo,
) -> Result<(), Vec<anyhow::Error>> {
    let notification = Notification {
        title: "Your VPN Account Has Been Created".to_owned(),
        send_to: info.username.clone(),
        by_methods: preferred_methods(&info.username),
        situation: Situation::AccountCreated,
        project: env.project.clone(),
        context: tera::Context::from_value(json!({
            "username": info.username,
            "password": info.password,
        }))
        .expect("Expected to create context for notification"),
        attachment: Some(AttachmentInfo {
            name: "os-vpn-client.ovpn".to_owned(),
            path: PathBuf::from("./config_data/os-vpn-client.ovpn"),
        }),
    };

    let mut errors: Vec<anyhow::Error> = Vec::new();

    match send(env, notification).await {
        Ok(_) => {}
        Err(e) => {
            tracing::error!(
                "Failed to send email to {:?} with error {e:#?}",
                info.username
            );
            errors.push(e)
        }
    }

    if (errors.is_empty()) {
        return Ok(());
    } else {
        return Err(errors);
    }
}

pub struct DefaultVpnInfo {
    user: Username,
    username: String,
    password: String,
    for_project: String,
}

pub struct AccessInfo {
    for_user: String,
    for_project: String,
}

// pub fn access_granted(env: &Env, info: &AccessInfo) {
//     //
// }

// pub fn access_removed(env: &Env, info: &AccessInfo) {
//     //
// }

static TERA: once_cell::sync::Lazy<tera::Tera> =
    once_cell::sync::Lazy::new(|| match Tera::new("templates/**/*.html") {
        Ok(t) => t,
        Err(e) => {
            panic!("Couldn't create templating instance, failure: {e}")
        }
    });

pub mod templates {
    pub fn retrieve(
        project: String,
        situation: Situation,
        mode: RenderTarget,
    ) -> Result<String, anyhow::Error> {
        tracing::info!("List of templates available:");
        let tns = TERA.get_template_names().collect_vec();
        tracing::info!("{tns:?}");
        // crunch the filter, return Ok(name of the template in tera)
        Ok(settings().projects.get(project.as_str())
                            .ok_or(anyhow::Error::msg(format!("no project by name {project}")))?
                            .notifications.get(&mode)
                            .ok_or(anyhow::Error::msg(format!("project {project} does not support render mode {mode:?}")))?
                            .get(&situation)
                            .ok_or(anyhow::Error::msg(format!("project {project} with render mode {mode:?} does not support situation {situation:?}")))?.to_owned())
    }

    use std::{collections::HashMap, path::PathBuf};

    use common::prelude::{anyhow, itertools::Itertools, tracing};
    use config::{settings, RenderTarget, Situation};

    use crate::TERA;

    pub struct Template {
        file: std::path::PathBuf,
    }

    pub struct Projects {}

    pub struct SituationalTemplate {
        by_render_mode: HashMap<RenderMode, Template>,
    }

    #[derive(Debug, Clone)]
    pub enum RenderMode {
        //#[]
        HTML,
        PlainText,
    }

    pub type TemplateName = String;

    pub struct Project {
        //project_name: String,
        by_situation: HashMap<TemplateName, SituationalTemplate>,
    }

    pub type ProjectName = String;

    pub struct ConfigFile {
        by_project: HashMap<ProjectName, Project>,
    }
}
