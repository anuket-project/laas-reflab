//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::*;

use aide::axum::{
    routing::{get, post},
    ApiRouter,
};
use aide::OperationIo;
use axum::{
    extract::Path,
    response::{IntoResponse, Response},
    Json,
};
use axum_macros::debug_handler;
use dal::{new_client, AsEasyTransaction, DBTable, ExistingRow, FKey};
use models::dal::web::*;
use models::dashboard::{Aggregate, LifeCycleState};
use schemars::JsonSchema;
use thiserror::Error;

use axum::http::StatusCode;
use uuid::Uuid;
use workflows::entry::DISPATCH;

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
#[derive(Error, Debug, Clone, OperationIo, JsonSchema)]
pub enum UserApiError {
    #[error("Error getting committing or initializing database transaction.")]
    DatabaseTransaction,
    #[error("Error retrieving database client.")]
    DatabaseClient,
    #[error("Aggregate does not exist.")]
    InvalidId,
    #[error("Error dispatching add user task.")]
    Dispatch,
    #[error("Empty or malformed user in users field.")]
    EmptyUser,
    #[error("Aggregate has not finished provisioning.")]
    AggregateNotReady,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, OperationIo)]
pub struct AddUserRequestResponse {
    pub users: Vec<String>,
}

impl IntoResponse for UserApiError {
    fn into_response(self) -> Response {
        let (status, err_msg) = match self {
            UserApiError::EmptyUser | UserApiError::InvalidId | UserApiError::AggregateNotReady => {
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            UserApiError::DatabaseClient
            | UserApiError::Dispatch
            | UserApiError::DatabaseTransaction => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };

        (status, Json(serde_json::json!({ "message": err_msg }))).into_response()
    }
}

fn validate_user(user: &str) -> Result<(), UserApiError> {
    if user.trim().is_empty() {
        Err(UserApiError::EmptyUser)
    } else {
        Ok(())
    }
}

#[debug_handler]
pub async fn add_users_to_booking(
    Path(id): Path<Uuid>,
    Json(request): Json<AddUserRequestResponse>,
) -> Result<Json<AddUserRequestResponse>, UserApiError> {
    let mut aggregate_row: ExistingRow<Aggregate> = fetch_aggregate(&id).await?;

    for user in &request.users {
        validate_user(user)?;
    }

    let mut client = new_client()
        .await
        .map_err(|_| UserApiError::DatabaseClient)?;
    let mut transaction = client
        .easy_transaction()
        .await
        .map_err(|_| UserApiError::DatabaseTransaction)?;

    let mut new_users: Vec<String> = vec![];

    for item in request.users {
        if !aggregate_row.users.contains(&item) {
            aggregate_row.users.push(item.clone());
            new_users.push(item);
        }
    }

    aggregate_row
        .update(&mut transaction)
        .await
        .map_err(|_| UserApiError::DatabaseTransaction)?;

    transaction
        .commit()
        .await
        .map_err(|_| UserApiError::DatabaseTransaction)?;

    let agg_id = FKey::from_id(aggregate_row.id());

    let dispatch = DISPATCH.get().ok_or(UserApiError::Dispatch)?;

    dispatch
        .send(workflows::entry::Action::AddUsers {
            agg_id,
            users: new_users.clone(),
        })
        .map_err(|_| UserApiError::Dispatch)?;

    Ok(Json(AddUserRequestResponse {
        users: aggregate_row.users.clone(),
    }))
}

async fn fetch_aggregate(id: &Uuid) -> Result<ExistingRow<Aggregate>, UserApiError> {
    let mut client = new_client()
        .await
        .map_err(|_| UserApiError::DatabaseClient)?;

    let mut transaction = client
        .easy_transaction()
        .await
        .map_err(|_| UserApiError::DatabaseTransaction)?;

    let aggregate_row = Aggregate::select()
        .where_field("id")
        .equals::<models::dal::ID>((*id).into())
        .run(&mut transaction)
        .await
        .map_err(|_| UserApiError::InvalidId)?
        .pop()
        .ok_or(UserApiError::InvalidId)?;

    transaction
        .commit()
        .await
        .map_err(|_| UserApiError::DatabaseTransaction)?;

    if !(aggregate_row.state == LifeCycleState::Active) {
        return Err(UserApiError::AggregateNotReady);
    }

    Ok(aggregate_row)
}

pub fn routes(state: AppState) -> ApiRouter {
    ApiRouter::new()
        .route("/:username", get(get_user))
        .route("/create", post(create_user))
        .route("/:username/ssh", post(set_ssh))
        .route("/:username/company", post(set_company))
        .route("/:username/email", post(set_email))
        .route("/:aggregate_id/addusers", post(add_users_to_booking))
}
