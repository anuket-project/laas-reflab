use aide::OperationIo;
use axum::{
    debug_handler, extract::{Json, Path}, http::StatusCode, response::{IntoResponse, Response}
};
use common::prelude::serde_json;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, info, span::Id, warn};

use uuid::Uuid;

use models::{
    dal::{new_client, AsEasyTransaction, DBTable, ExistingRow},
    dashboard::{Aggregate, Instance, LifeCycleState},
    inventory::Host,
};
use workflows::{deploy_booking::set_host_power_state::{
    get_host_power_state, set_host_power_state, HostConfig, PowerState, PowerStateError,
    TimeoutConfig,
}, entry::{Action, DISPATCH}};

/// Respective error types for the handlers. All of these error messages will be converted into an
/// HTTP response.
#[derive(Debug, Error, Deserialize, Serialize, JsonSchema, OperationIo)]
pub enum ApiPowerStateError {
    #[error("Invalid instance ID")]
    InvalidInstanceId,

    #[error("No linked hosts")]
    NoLinkedHosts,

    #[error("Database client error")]
    DatabaseClient,

    #[error("Database transaction error")]
    DatabaseTransaction,

    #[error("Cannot perform operation on an inactive host")]
    InactiveHost,

    #[error("IPMI operation failed: {0}")]
    IpmiOperationFailed(#[from] PowerStateError),

    #[error("FQDN error: {0}")]
    FQDNError(String),
}

/// Converts the errors into their respective HTTP responses.
impl IntoResponse for ApiPowerStateError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ApiPowerStateError::InvalidInstanceId
            | ApiPowerStateError::NoLinkedHosts
            | ApiPowerStateError::FQDNError(_)
            | ApiPowerStateError::InactiveHost => (StatusCode::BAD_REQUEST, self.to_string()),

            ApiPowerStateError::DatabaseTransaction | ApiPowerStateError::DatabaseClient => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }

            ApiPowerStateError::IpmiOperationFailed(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };
        (
            status,
            Json(serde_json::json!({ "message": error_message })),
        )
            .into_response()
    }
}

/// All the possible power commands to send to a host.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub enum PowerCommand {
    PowerOff,
    PowerOn,
    Restart,
}

/// The request payload for the power control handler, sent as JSON.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PowerCommandRequest {
    /// The power command to be executed.
    pub command: PowerCommand,
    #[serde(default)]
    /// The timeout configuration for the IPMI command.
    pub timeout_config: TimeoutConfig,
}

/// The response payload for the power control handler, returned as JSON.
#[derive(Debug, Serialize, Deserialize, JsonSchema, OperationIo)]
pub struct PowerStateResponse {
    pub power_state: PowerState,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, OperationIo)]
pub struct IPMIFQDNResponse {
    pub ipmi_fqdn: String,
}

/// A handler that retrieves the current power state of a specific instance.
///
/// This handler is responsible for obtaining the power state of a host machine associated
/// with a given instance. It first fetches the instance details from the database and then
/// retrieves the host information. The power state is obtained using IPMI commands executed
/// against the host.
///
/// # Arguments
///
/// * `Path(instance_id)` - An [`Uuid`] representing an instance id as a [`Path`] parameter.
///
/// # Returns
///
/// This function returns a [`Result`] that wraps [`PowerStateResponse`] as [`Json`] or an [`ApiPowerStateError`].
///
pub async fn instance_power_state(
    Path(instance_llid): Path<Uuid>,
) -> Result<Json<PowerStateResponse>, ApiPowerStateError> {
    info!("Fetching power state for instance ID: {:?}", instance_llid);

    let instance = fetch_instance(&instance_llid).await?;

    if !is_instance_active(&instance).await? {
        error!("Cannot perform operation on an inactive host");
        return Err(ApiPowerStateError::InactiveHost);
    }

    if let Some(host) = fetch_host(&instance).await? {
        info!("Fetching power state from host: {}", host.ipmi_fqdn);
        let power_state = get_host_power_state(&HostConfig::try_from(host)?).await?;

        Ok(Json(PowerStateResponse { power_state }))
    } else {
        warn!("No host linked to instance ID: {}", instance_llid);
        Err(ApiPowerStateError::NoLinkedHosts)
    }
}

/// Handler to control the power state of an instance.
///
/// This handler processes a power command (like power on, off, or restart) for a specific instance.
/// It involves fetching the instance details, obtaining the linked host information, and then
/// sending the appropriate IPMI command to change the power state of the host machine.
///
/// # Arguments
///
/// * `Path(instance_id)` - A [`LLID`] representing an instance id as a [`Path`] parameter.
/// * `Json(request)` - A JSON payload that is deserialized into [`PowerCommandRequest`] representing the desired power command.
///
/// # Returns
///
/// This function returns a [`Result`] that wraps [`PowerStateResponse`] as [`Json`] or an [`ApiPowerStateError`].
#[axum::debug_handler]
pub async fn instance_power_control(
    Path(instance_llid): Path<Uuid>,
    Json(request): Json<PowerCommandRequest>,
) -> Result<Json<PowerStateResponse>, ApiPowerStateError> {
    info!(
        "Attempting {:?} command for instance ID: {:?}",
        request.command, instance_llid
    );

    // Fetch the instance from the database
    let instance = fetch_instance(&instance_llid).await?;

    if !is_instance_active(&instance).await? {
        error!("Cannot perform operation on an inactive host");
        return Err(ApiPowerStateError::InactiveHost);
    }

    if let Some(host) = fetch_host(&instance).await? {
        // Determine the desired power state based on the command
        let desired_state = match request.command {
            PowerCommand::PowerOff => PowerState::Off,
            PowerCommand::PowerOn => PowerState::On,
            PowerCommand::Restart => PowerState::Reset,
        };

        let power_state = set_host_power_state(
            &HostConfig::try_from(host)?,
            request.timeout_config,
            desired_state,
        )
        .await?;

        Ok(Json(PowerStateResponse { power_state }))
    } else {
        error!("No host linked to instance ID: {}", instance_llid);
        Err(ApiPowerStateError::NoLinkedHosts)
    }
}

/// Fetches the an [`Instance`] from the database based on the given instance ID.
///
/// # Arguments
///
/// * `instance_id` - A reference to an [`LLID`] representing the instance ID to be fetched.
///
/// # Returns
///
/// This function returns a [`Result`] of [`Instance`] or an [`ApiPowerStateError`].
pub async fn fetch_instance(instance_id: &Uuid) -> Result<Instance, ApiPowerStateError> {
    debug!("Fetching instance from database, ID: {:?}", instance_id);
    let mut client = new_client()
        .await
        .map_err(|_| ApiPowerStateError::DatabaseClient)?;
    let mut transaction = client
        .easy_transaction()
        .await
        .map_err(|_| ApiPowerStateError::DatabaseTransaction)?;

    let instance_row: ExistingRow<Instance> = Instance::select()
        .where_field("id")
        .equals::<models::dal::ID>((*instance_id).into())
        .run(&mut transaction)
        .await
        .map_err(|_| ApiPowerStateError::InvalidInstanceId)?
        .pop()
        .ok_or(ApiPowerStateError::InvalidInstanceId)?;

    transaction
        .commit()
        .await
        .map_err(|_| ApiPowerStateError::DatabaseTransaction)?;

    Ok(instance_row.into_inner())
}

/// Fetches a [`Host`] from the database based on the given [`Instance`] reference.
///
/// # Arguments
///
/// * `instance` -  A reference to an [`Instance`] for which the host needs to be fetched.
///
/// # Returns
///
/// This function returns a [`Result`] that wraps an [`Option`] of [`Host`] or an [`ApiPowerStateError`].
pub async fn fetch_host(instance: &Instance) -> Result<Option<Host>, ApiPowerStateError> {
    debug!("Fetching host for instance ID: {:?}", instance.id);
    if let Some(host_key) = &instance.linked_host {
        let mut client = new_client()
            .await
            .map_err(|_| ApiPowerStateError::DatabaseClient)?;
        let mut transaction = client
            .easy_transaction()
            .await
            .map_err(|_| ApiPowerStateError::DatabaseTransaction)?;

        let host = host_key
            .get(&mut transaction)
            .await
            .map_err(|_| ApiPowerStateError::DatabaseTransaction)?
            .into_inner();

        transaction
            .commit()
            .await
            .map_err(|_| ApiPowerStateError::DatabaseTransaction)?;

        Ok(Some(host))
    } else {
        Ok(None)
    }
}

/// Fetches the linked [`Aggregate`] from a reference to an [`Instance`]
/// and returns `Ok(true)` if the aggregate is in [`LifeCycleState::Active`].
/// Assumes the [`Instance`] type has a field `aggregate` which is a foreign key to an [`Aggregate`].
///
/// # Arguments
///
/// * `instance` - An [`Instance`] reference for which to check the linked aggregate's state.
///
/// # Returns
///
/// This function returns a [`Result`] that wraps a `bool` indicating if the linked aggregate
/// is in the `Active` state, or an [`ApiPowerStateError`] in the case of an error.
async fn is_instance_active(instance: &Instance) -> Result<bool, ApiPowerStateError> {
    let mut client = new_client()
        .await
        .map_err(|_| ApiPowerStateError::DatabaseClient)?;
    let mut transaction = client
        .easy_transaction()
        .await
        .map_err(|_| ApiPowerStateError::DatabaseTransaction)?;

    // fetch the linked Aggregate based on the foreign key from the instance
    let aggregate: ExistingRow<Aggregate> = instance
        .aggregate
        .get(&mut transaction)
        .await
        .map_err(|_| ApiPowerStateError::DatabaseTransaction)?;

    transaction
        .commit()
        .await
        .map_err(|_| ApiPowerStateError::DatabaseTransaction)?;

    // check if the aggregate's life cycle state is `Active`
    Ok(aggregate.into_inner().state == LifeCycleState::Active)
}

pub async fn fetch_ipmi_fqdn(Path(instance_id): Path<Uuid>) -> Result<Json<IPMIFQDNResponse>, ApiPowerStateError> {
    let host = fetch_host(&fetch_instance(&instance_id).await?)
        .await?
        .unwrap();
    Ok(Json(IPMIFQDNResponse { ipmi_fqdn: host.ipmi_fqdn }))
}