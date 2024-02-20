//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::*;

use aide::axum::{
    routing::{get, post},
    ApiRouter,
};
use axum::{extract::Path, Json};
use models::dal::web::*;

use axum::http::StatusCode;

use super::{AppState, WebError};

// check ipa Acct
// create ipa acct
// set ssh key
// set company
// set email

use users::ipa::*;

pub async fn get_user(Path(username): Path<String>) -> Result<Json<User>, WebError> {
    let mut ipa = IPA::init()
        .await
        .log_server_error("Failed to connect to IPA", true)?;
    Ok(Json(
        ipa.find_matching_user(username, true, false)
            .await
            .log_server_error("Failed to find user", true)?,
    ))
}

pub async fn create_user(Json(user): Json<User>) -> Result<(), WebError> {
    let mut ipa = IPA::init()
        .await
        .log_server_error("Failed to connect to IPA", true)?;
    let res = ipa.create_user(user, false).await;

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
            Ok(())
        }
        Err(e) => {
            tracing::info!("Failed to create user with error: {e}");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create user, encountered error: {e}"),
            ))
        }
    }
}

pub async fn set_ssh(
    Path(username): Path<String>,
    Json(keys): Json<Vec<String>>,
) -> Result<(), WebError> {
    let mut ipa = IPA::init()
        .await
        .log_server_error("Failed to connect to IPA", true)?;

    ipa.update_user(
        username.clone(),
        vec![],
        vec![UserData::ipasshpubkey(None)],
        false,
    )
    .await
    .log_server_error("Failed to set key to none", true)?;

    for key in keys {
        ipa.update_user(
            username.clone(),
            vec![UserData::ipasshpubkey(Some(key))],
            vec![],
            false,
        )
        .await
        .log_server_error("Failed to add key", true)?;
    }

    Ok(())
}

pub async fn set_company(
    Path(username): Path<String>,
    Json(company): Json<String>,
) -> Result<(), WebError> {
    let mut ipa = IPA::init()
        .await
        .log_server_error("Failed to connect to IPA", true)?;
    ipa.update_user(username, vec![], vec![UserData::ou(Some(company))], false)
        .await
        .log_server_error("Failed to find user", true)?;
    Ok(())
}

pub async fn set_email(
    Path(username): Path<String>,
    Json(email): Json<String>,
) -> Result<(), WebError> {
    let mut ipa = IPA::init()
        .await
        .log_server_error("Failed to connect to IPA", true)?;
    ipa.update_user(username, vec![], vec![UserData::mail(Some(email))], false)
        .await
        .log_server_error("Failed to find user", true)?;
    Ok(())
}

pub async fn request_password_reset(Path(username): Path<String>) -> Result<(), WebError> {
    todo!("password resets")
}

pub fn routes(state: AppState) -> ApiRouter {
    ApiRouter::new()
        .route("/:username", get(get_user))
        .route("/create", post(create_user))
        .route("/:username/ssh", post(set_ssh))
        .route("/:username/company", post(set_company))
        .route("/:username/email", post(set_email))
}
