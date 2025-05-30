//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use crate::{render, Env, Notification};
use common::prelude::{
    config::*,
    reqwest::RequestBuilder,
    serde_json::{self, json},
    tracing, *,
};
use lettre::{
    message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart},
    transport::smtp::{authentication::Credentials, client::TlsParameters},
    *,
};
use std::{fs, path::PathBuf};
use users::*;

pub async fn send(env: &Env, notification: Notification) -> Result<(), anyhow::Error> {
    tracing::info!("Sending notification {notification:?} by email");
    let mut ipa = ipa::IPA::init().await.unwrap();
    tracing::info!("connected to IPA");
    let email_config = config::settings().notifications.clone();
    let user = ipa
        .find_matching_user(notification.send_to.clone(), false, false)
        .await
        .unwrap();
    let email = user.mail;
    let addr: Address = email.parse().expect("Couldn't parse addr");
    let from_addr: Email = email_config
        .send_from_email
        .expect("Expected a from email address!");
    let email = match &notification.attachment {
        Some(attachment) => Message::builder()
            .from(Mailbox::new(
                Some(from_addr.username.clone()),
                Address::new(from_addr.username.clone(), from_addr.domain.clone())
                    .expect("Expected to create address"),
            ))
            .to(Mailbox::new(None, addr))
            .subject(notification.title.clone())
            .multipart(
                MultiPart::mixed()
                    .singlepart(
                        SinglePart::builder().header(ContentType::TEXT_HTML).body(
                            render(&notification, config::RenderTarget::Email)
                                .expect("Expected to render template"),
                        ),
                    )
                    .singlepart(Attachment::new(attachment.name.clone()).body(
                        fs::read(attachment.path.clone()).unwrap(),
                        ContentType::TEXT_PLAIN,
                    )),
            )
            .expect("Expected to create email"),
        None => Message::builder()
            .from(Mailbox::new(
                Some(from_addr.username.clone()),
                Address::new(from_addr.username.clone(), from_addr.domain.clone())
                    .expect("Expected to create address"),
            ))
            .to(Mailbox::new(None, addr))
            .subject(notification.title.clone())
            .header(ContentType::TEXT_HTML)
            .body(
                render(&notification, config::RenderTarget::Email)
                    .expect("Expected to render template"),
            )
            .expect("Expected to create email"),
    };

    let mail_server = settings().notifications.mail_server.clone();
    let mailer = SmtpTransport::relay(mail_server.host.as_str())
        .unwrap()
        .port(mail_server.port)
        .tls(transport::smtp::client::Tls::None)
        .build();

    match mailer.send(&email) {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::Error::msg(e.to_string())),
    }
}

pub async fn send_to_admins(error: String) {
    if let Some(ec) = &config::settings().notifications.admin_mail_server {
        send_to_admins_email(error.clone()).await;
    }

    if let Some(gc) = &config::settings().notifications.admin_gchat_webhook {
        send_to_admins_gchat(error).await;
    }
}

pub async fn send_to_admins_gchat(error: String) {
    let client = reqwest::Client::new();
    let response = client
        .post(
            config::settings()
                .notifications
                .admin_gchat_webhook
                .clone()
                .unwrap(),
        )
        .header("Content-Type", "application/json")
        .json(&json!({"text": error}))
        .send()
        .await
        .expect("Expected to receive response")
        .text()
        .await
        .expect("Expected to receive payload.");
}

pub async fn send_to_admins_email(error: String) {
    let email_config = config::settings().notifications.clone();
    let from_addr = email_config
        .admin_send_from_email
        .expect("Expected an admin from email address!");
    let to_addr = email_config
        .admin_send_to_email
        .expect("Expected an admin to email address!");
    let email = Message::builder()
        .from(Mailbox::new(
            Some(from_addr.username.clone()),
            Address::new(from_addr.username.clone(), from_addr.domain.clone())
                .expect("Expected to create address"),
        ))
        .to(Mailbox::new(
            None,
            Address::new(to_addr.username.clone(), to_addr.domain.clone()).unwrap(),
        ))
        .subject("LibLaaS Error Encountered")
        .header(ContentType::TEXT_HTML)
        .body(error)
        .expect("Expected to create email");

    let mail_server = settings().notifications.admin_mail_server.clone().unwrap();
    let mailer = SmtpTransport::relay(mail_server.host.as_str())
        .unwrap()
        .port(mail_server.port)
        .tls(transport::smtp::client::Tls::None)
        .build();

    match mailer.send(&email) {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::Error::msg(e.to_string())),
    }
    .unwrap();
}

/// Sends an email to admins using an HTML template instead of plaintext
pub async fn send_to_admins_email_template(
    env: &Env,
    notification: Notification,
) -> Result<(), anyhow::Error> {
    tracing::info!("Sending notification {notification:?} to ADMINS by email");
    let mut ipa = ipa::IPA::init().await?;
    tracing::info!("connected to IPA");
    let email_config = config::settings().notifications.clone();
    let from_addr: Email = email_config
        .admin_send_from_email
        .expect("Expected a from email address!");

    let to_addr: Email = email_config
        .admin_send_to_email
        .expect("Expected an admin to email address!");

    let email = Message::builder()
        .from(Mailbox::new(
            Some(from_addr.username.clone()),
            Address::new(from_addr.username.clone(), from_addr.domain.clone())
                .expect("Expected to create address"),
        ))
        .to(Mailbox::new(
            None,
            Address::new(to_addr.username.clone(), to_addr.domain.clone()).unwrap(),
        ))
        .subject(notification.title.clone())
        .header(ContentType::TEXT_HTML)
        .body(
            render(&notification, config::RenderTarget::Email)
                .expect("Expected to render template"),
        )
        .expect("Expected to create email");

    let mail_server = settings().notifications.mail_server.clone();
    let mailer = SmtpTransport::relay(mail_server.host.as_str())?
        .port(mail_server.port)
        .tls(transport::smtp::client::Tls::None)
        .build();

    match mailer.send(&email) {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::Error::msg(e.to_string())),
    }
}
