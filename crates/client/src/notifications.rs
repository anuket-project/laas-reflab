use crate::remote::{Select, Server, Text};
use common::prelude::{anyhow, config::Situation, tracing};
use config::settings;
use notifications::{
    email::{send_to_admins_email, send_to_admins_gchat},
    send_test_email,
};
use tascii::prelude::Runtime;

use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};

#[derive(Display, Clone, EnumString, EnumIter, Debug)]
pub enum NotificationActions {
    #[strum(serialize = "Test Email Template")]
    TestEmailTemplate,
    #[strum(serialize = "Send Admin Google Chat")]
    SendAdminGoogleChat,
    #[strum(serialize = "Send Admin Email")]
    SendAdminEmail,
}

pub async fn notification_actions(
    session: &Server,
    _tascii_rt: &'static Runtime,
) -> Result<(), anyhow::Error> {
    let choice = Select::new("Select an action:", NotificationActions::iter().collect())
        .prompt(session)
        .unwrap();

    match choice {
        NotificationActions::TestEmailTemplate => handle_test_email_template(session).await,
        NotificationActions::SendAdminGoogleChat => handle_send_admin_chat(session).await,
        NotificationActions::SendAdminEmail => handle_send_admin_email(session).await,
    }
}

async fn handle_test_email_template(session: &Server) -> Result<(), anyhow::Error> {
    let status = Select::new(
        "Select a Status: ",
        vec![
            Situation::BookingCreated,
            Situation::BookingExpiring,
            Situation::BookingExpired,
            Situation::RequestBookingExtension,
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
        Ok(_) => tracing::info!("Successfully sent email to {:?}", &dest_user),
        Err(e) => tracing::error!("Failed to send email to {:?}: {:?}", &dest_user, e),
    }
    Ok(())
}

async fn handle_send_admin_email(session: &Server) -> Result<(), anyhow::Error> {
    let email = Text::new("Message: ").prompt(session).unwrap();
    send_to_admins_email(email).await;
    Ok(())
}

async fn handle_send_admin_chat(session: &Server) -> Result<(), anyhow::Error> {
    let gchat = Text::new("Message: ").prompt(session).unwrap();
    send_to_admins_gchat(gchat).await;
    Ok(())
}
