//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::{strum_macros::Display, tracing};
use dal::{new_client, AsEasyTransaction, FKey, ID};

use models::inventory::Host;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tascii::{prelude::*, task_trait::AsyncRunnable};

use std::str::{self};
use thiserror::Error;
use tokio::process::Command;
use tokio::time::{sleep, timeout, Duration};
use tracing::{debug, error, info, warn};

use crate::{
    deploy_booking::reachable::WaitReachable,
    utils::net::{validate_fqdn, validate_ip},
};

#[derive(Serialize, Deserialize, Debug, Hash, Clone, Eq, PartialEq)]
pub struct SetPower {
    pub host: FKey<Host>,
    pub pstate: PowerState,
}

/// All the possible power states of a host.
#[derive(Serialize, Deserialize, Debug, Hash, Clone, Eq, PartialEq, Display, JsonSchema)]
pub enum PowerState {
    On,
    Off,
    Reset,
    Unknown,
}

/// A task that sets the power state of a host.
impl SetPower {
    pub fn off(host: FKey<Host>) -> Self {
        tracing::warn!("In setpower::off.");
        Self {
            host,
            pstate: PowerState::Off,
        }
    }

    pub fn on(host: FKey<Host>) -> Self {
        tracing::warn!("In setpower::on.");
        Self {
            host,
            pstate: PowerState::On,
        }
    }

    pub fn reset(host: FKey<Host>) -> Self {
        tracing::warn!("In setpower::on.");
        Self {
            host,
            pstate: PowerState::Reset,
        }
    }
}

tascii::mark_task!(SetPower);
impl AsyncRunnable for SetPower {
    type Output = ();

    //Return true if succeeded, else false

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        // std::thread::sleep(Duration::from_secs_f64(5.0)); // TODO: get rid of wait
        tracing::warn!("Setting power.");
        let host = self.host.get(&mut transaction).await.unwrap();
        transaction.commit().await.unwrap();

        let ipmi_fqdn = &host.ipmi_fqdn;
        let ipmi_admin_user = &host.ipmi_user;
        let ipmi_admin_password = &host.ipmi_pass;

        // make sure we can reach the IPMI endpoint
        tracing::info!("Checking that we can reach the IPMI endpoint");
        let ipmi_url = context
            .spawn(WaitReachable {
                endpoint: ipmi_fqdn.clone(),
                timeout: Duration::from_secs(120),
            })
            .join()?;

        tracing::info!(
            "about to run ipmi power on {:?} to set power to {:?}",
            ipmi_fqdn,
            self.pstate
        );

        let ipmitool = std::process::Command::new("ipmitool")
            .args([
                "-I",
                "lanplus",
                "-C",
                "3",
                "-H",
                &ipmi_url,
                "-U",
                ipmi_admin_user,
                "-P",
                ipmi_admin_password,
                "chassis",
                "power",
                match self.pstate {
                    PowerState::Off => "off",
                    PowerState::On => "on",
                    PowerState::Reset => "reset",
                    PowerState::Unknown => panic!("bad instruction"),
                },
            ])
            .output()
            .expect("Failed to execute ipmitool command");
        let stdout = String::from_utf8(ipmitool.stdout).expect("no stdout?");
        let stderr = String::from_utf8(ipmitool.stderr).expect("no stderr?");

        tracing::info!("ran ipmitool, output was: {stdout}, with stderr: {stderr}");

        if stderr.contains("Unable to establish IPMI") {
            return Err(TaskError::Reason(format!(
                "IPMItool could not reach the host, host was: {}, fqdn was: {ipmi_fqdn}",
                host.server_name
            )));
        }

        for _ in 0..50 {
            tracing::info!("about to check host power state");
            std::thread::sleep(Duration::from_secs_f64(5.0));
            tracing::info!("checking host power state");
            let current_state = get_host_power_state(
                &HostConfig::try_from(&host.clone().into_inner()).map_err(|e| {
                    error!("Invalid parameters or fqdn! {e}");
                    TaskError::Reason("Invalid parameters or fqdn!".to_string())
                })?,
            )
            .await
            .map_err(|e| {
                tracing::error!("Error getting host power state: {:?}", e);
                TaskError::Reason(format!("Error getting host power state: {:?}", e))
            })?;

            if current_state.eq(&self.pstate) || self.pstate.eq(&PowerState::Reset) {
                tracing::info!("Host reached desired state! :)");
                return Ok(());
            } else {
                match current_state {
                    PowerState::On => {
                        tracing::warn!("Host is not in desired state, is on instead. :(");
                    }
                    PowerState::Off => {
                        tracing::warn!("Host is not in desired state, is off instead. :(");
                    }
                    PowerState::Reset => {
                        continue;
                    }
                    PowerState::Unknown => {
                        continue;
                    }
                }
            }
        }

        Err(TaskError::Reason(format!("host {} with ipmi fqdn {ipmi_fqdn} failed to reach desired power state, even though it accepted the power command", host.server_name)))
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("SetPowerState").versioned(1)
    }

    fn summarize(&self, id: ID) -> String {
        format!("[{id} | Set Power]")
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(240.0)
    }
}

/// Errors that can occur while setting or getting the power state.
#[derive(Debug, Error, Serialize, Deserialize, JsonSchema)]
pub enum PowerStateError {
    #[error("Unknown power state cannot be set")]
    SetUnknown,
    #[error("Command execution failed: {0}")]
    CommandExecutionFailed(String),
    #[error("Command returned a non-zero exit status: {0}, {1}")]
    CommandNonZeroExitStatus(i32, String),
    #[error("Timeout reached while waiting for power state change")]
    TimeoutReached,
    #[error("Invalid input parameter: {0}")]
    InvalidInputParameter(String),
    #[error("UTF-8 decoding error: {0}")]
    Utf8Error(String),
    #[error("Unknown power state, can't infer from output: {0}")]
    UnknownPowerState(String),
    #[error("Host {0} is unreachable")]
    HostUnreachable(String),
}

/// Configuration parameters for interacting with a host using IPMI.
///
/// This struct stores the Fully Qualified Domain Name (FQDN), along with the username and password
/// required for IPMI authentication.
///
/// # Examples
///
/// ```
/// use workflows::deploy_booking::set_host_power_state::HostConfig;
///
/// let config = HostConfig {
///     fqdn: "example.domain.local".to_string(),
///     user: "admin".to_string(),
///     password: "password123".to_string(),
/// };
/// ```
pub struct HostConfig {
    /// The Fully Qualified Domain Name of the IPMI for the host.
    pub fqdn: String,
    /// The IPMI username of the host.
    pub user: String,
    /// The IPMI password of the host.
    pub password: String,
}

/// Configuration for timeouts and retries in IPMI power functions.
///
/// Specifies the number of retries, the delay between retries, and the total timeout duration
/// for IPMI operations.
///
/// # Examples
///
/// ```rust
/// use workflows::deploy_booking::set_host_power_state::TimeoutConfig;
/// use tokio::time::Duration;
///
/// let config = TimeoutConfig {
///     max_retries: 3,
///     retry_interval: 5,
///     timeout_duration: 30,
/// };
///
/// let config_default = TimeoutConfig::default();
///
/// assert_eq!(config, config_default);
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct TimeoutConfig {
    #[serde(default = "default_num_retries")]
    /// The maximum number of retries for IPMI operations.
    pub max_retries: u8,
    #[serde(default = "default_retry_interval")]
    /// The delay between retries for IPMI operations in seconds.
    pub retry_interval: u8,
    #[serde(default = "default_timeout_duration")]
    /// The total timeout duration for IPMI operations in seconds.
    pub timeout_duration: u16,
}

impl TimeoutConfig {
    pub fn new(max: u8, interval: u8, timeout: Option<u16>) -> TimeoutConfig {
        let t = match timeout {
            Some(i) => i,
            None => max as u16 * interval as u16,
        };

        TimeoutConfig {
            max_retries: max,
            retry_interval: interval,
            timeout_duration: t,
        }
    }
}

/// Default number of retries for IPMI operations.
pub const fn default_num_retries() -> u8 {
    3 // Attempts
}

/// Default delay between retries for IPMI operations.
pub const fn default_retry_interval() -> u8 {
    5 // Seconds
}

/// Default timeout duration for IPMI operations.
pub const fn default_timeout_duration() -> u16 {
    30 // Seconds
}

/// Implements the `Default` trait for `TimeoutConfig`.
/// The default values are defined by constant functions in this module.
impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            max_retries: default_num_retries(),
            retry_interval: default_retry_interval(),
            timeout_duration: default_timeout_duration(),
        }
    }
}

/// Implements conversion from [`Host`] to [`HostConfig`].
///
/// This implementation validates the `ipmi_fqdn`, `ipmi_user`, and `ipmi_pass` from the [`Host`] struct
/// by calling [`validate_fqdn()`] and [`validate_input()`] on the respective parameters and returns a [`HostConfig`]
/// on [`Ok`].
///
/// # Errors
///
/// This implementation can fail if:
///
/// - If `ipmi_fqdn`, `ipmi_user`, or `ipmi_pass` are empty or contain control characters.
/// - `ipmi_fqdn` is not a valid domain name. Ex: `example..domain12.l!23@<`
///
/// # Examples
///
/// ```rust,no_run
/// use workflows::deploy_booking::set_host_power_state::HostConfig;
/// use dal::inventory::Host;
/// use dal::{
///     new_client,
///     web::*,
///     AsEasyTransaction,
///     DBTable,
///     ExistingRow
/// },
/// use std::convert::TryFrom;
///
/// #[tokio::main]
/// async fn main() {
///    // Assume host_uuid is obtained somehow
///    let host_uuid = LLID::from_str("00000000-0000-0000-0000-000000000000").unwrap();
///
///    let mut client = new_client().await.unwrap();
///    let mut transaction = client.easy_transaction().await.unwrap();
///
///    // Fetch the host directly using its UUID
///    let host = Host::select()
///        .where_field("id")
///        .equals::<models::dal::ID>(host_uuid.into())
///        .run(&mut transaction)
///        .await
///        .unwrap()
///        .pop()
///        .unwrap();
///
///    transaction.commit().await.unwrap();
///
///    // Convert the Host to HostConfig
///    let config = HostConfig::try_from(host.into_inner()).unwrap();
///
/// }
/// ```
///
impl TryFrom<Host> for HostConfig {
    type Error = PowerStateError;

    fn try_from(host: Host) -> Result<Self, Self::Error> {
        validate_input(&host.ipmi_fqdn)?;
        validate_input(&host.ipmi_user)?;
        validate_input(&host.ipmi_pass)?;

        let result = validate_fqdn(&host.ipmi_fqdn);
        if result.is_ok() || validate_ip(&host.ipmi_fqdn) {
            Ok(Self {
                fqdn: host.ipmi_fqdn,
                user: host.ipmi_user,
                password: host.ipmi_pass,
            })
        } else {
            Err(Self::Error::InvalidInputParameter(format!(
                "{} is invalid. {}",
                host.ipmi_fqdn,
                result.unwrap_err()
            )))
        }
    }
}

/// Implements conversion from a reference to a [`Host`] into a [`HostConfig`].
///
/// This implementation validates the `ipmi_fqdn`, `ipmi_user`, and `ipmi_pass` from the [`Host`] struct
/// by calling [`validate_fqdn()`] and [`validate_input()`] on the respective parameters and returns a [`HostConfig`]
/// on [`Ok`].
/// # Errors
///
/// This implementation can fail if:
///
/// - If `ipmi_fqdn`, `ipmi_user`, or `ipmi_pass` are empty or contain control characters.
/// - `ipmi_fqdn` is not a valid domain name. Ex: `example..domain12.l!23@<`
///
/// # Examples
///
/// ```rust,no_run
/// use workflows::deploy_booking::set_host_power_state::HostConfig;
/// use dal::inventory::Host;
/// use dal::models::LLID;
/// use dal::{
///     new_client,
///     web::*,
///     AsEasyTransaction,
///     DBTable,
///     ExistingRow
/// },
/// use std::convert::TryFrom;
///
/// #[tokio::main]
/// async fn main() {
///    // Assume host_uuid is obtained somehow
///    let host_uuid = LLID::from_str("00000000-0000-0000-0000-000000000000").unwrap();
///
///    let mut client = new_client().await.unwrap();
///    let mut transaction = client.easy_transaction().await.unwrap();
///
///    // Fetch the host directly using its UUID
///    let host = Host::select()
///        .where_field("id")
///        .equals::<models::dal::ID>(host_uuid.into())
///        .run(&mut transaction)
///        .await
///        .unwrap()
///        .pop()
///        .unwrap();
///
///    transaction.commit().await.unwrap();
///
///    // Convert the Host to HostConfig
///    let config = HostConfig::try_from(&host.into_inner()).unwrap();
///
/// }
/// ```
///
impl TryFrom<&Host> for HostConfig {
    type Error = PowerStateError;

    fn try_from(host: &Host) -> Result<Self, Self::Error> {
        validate_input(&host.ipmi_fqdn)?;
        validate_input(&host.ipmi_user)?;
        validate_input(&host.ipmi_pass)?;

        let result = validate_fqdn(&host.ipmi_fqdn);
        if result.is_ok() || validate_ip(&host.ipmi_fqdn) {
            Ok(Self {
                fqdn: host.ipmi_fqdn.clone(),
                user: host.ipmi_user.clone(),
                password: host.ipmi_pass.clone(),
            })
        } else {
            Err(Self::Error::InvalidInputParameter(format!(
                "{} is invalid. {}",
                host.ipmi_fqdn,
                result.unwrap_err()
            )))
        }
    }
}

/// Asynchronously waits for a host to become reachable via ping.
///
/// This function attempts to ping the host identified by `fqdn` until it becomes reachable or
/// until the timeout specified in `timeout_config` is reached. The function checks for host
/// reachability every `retry_interval` seconds for a maximum of `max_retries`.
///
/// # Arguments
///
/// * `fqdn` - A string slice that holds the Fully Qualified Domain Name of the host.
/// * `timeout_config` - A reference to [`TimeoutConfig`] specifying the timeout settings.
///
/// # Returns
///
/// Returns [`Ok`] if the host becomes reachable within the specified timeout and retries or [`PowerStateError`]
/// if the host remains unreachable after all retries within the
/// timeout or if the ping command fails to execute.
///
/// # Examples
///
/// ```rust
/// use workflows::deploy_booking::set_host_power_state::{
///     wait_for_reachable,
///     TimeoutConfig
/// };
/// use tokio::time::Duration;
///
/// #[tokio::main]
/// async fn main() {
///     let fqdn = "example.domain.local";
///     let timeout_config = TimeoutConfig::default();
///
///     match wait_for_reachable(fqdn, &timeout_config).await {
///         Ok(_) => println!("Host is reachable."),
///         Err(e) => println!("Failed to reach host: {:?}", e),
///     }
/// }
/// ```
///
/// # Errors
///
/// - [`PowerStateError::CommandExecutionFailed`] if the ping command fails to execute.
///
/// - [`PowerStateError::HostUnreachable`] if the host is not reachable within the specified timeout and retries.
///
pub async fn wait_for_reachable(
    fqdn: &str,
    timeout_config: &TimeoutConfig,
) -> Result<(), PowerStateError> {
    let end =
        tokio::time::Instant::now() + Duration::from_secs(timeout_config.timeout_duration as u64);
    let retry_count = 0;
    while tokio::time::Instant::now() < end && retry_count < timeout_config.max_retries {
        tokio::time::sleep(Duration::from_secs(timeout_config.retry_interval as u64)).await;
        let res = tokio::process::Command::new("ping")
            .args(["-c", "1", "-n", "-q", fqdn])
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| PowerStateError::CommandExecutionFailed(e.to_string()))?;

        if res.status.success() {}
    }
    Err(PowerStateError::HostUnreachable(fqdn.into()))
}

/// Validates an input string to ensure it is suitable for use in commands.
///
/// This function checks the input string to ensure it is not empty and does not contain
/// any control characters, which might lead to unintended effects or security issues.
///
/// # Arguments
///
/// * `input` - A string slice representing the input to be validated.
///
/// # Returns
///
/// Returns [`Ok`] or [`PowerStateError::InvalidInputParameter`] if the input is invalid.
///
pub fn validate_input(input: &str) -> Result<(), PowerStateError> {
    if input.is_empty() || input.contains(char::is_control) {
        return Err(PowerStateError::InvalidInputParameter(input.to_string()));
    }
    Ok(())
}

/// Sets the power state of a host.
///
/// This asynchronous function takes a reference to [`HostConfig`], a [`TimeoutConfig`],
/// and a desired [`PowerState`], and attempts to set the power state of the host accordingly
/// using [`execute_power_command()`] and [`confirm_power_state()`].
///
/// # Arguments
///
/// * `config` - A reference to [`HostConfig`] containing the host's details.
/// * `timeout_config` - A [`TimeoutConfig`] specifying the timeout settings.
/// * `desired_state` - The [`PowerState`] that we want to set for the host.
///
/// # Returns
///
/// Returns a [`Result`] of [`PowerState`] indicating the power state was successfully set to the desired state,
/// or an [`Err`] that wraps [`PowerStateError`] if there were issues in setting the power state.
///
/// # Examples
///
/// ```rust
///use workflows::deploy_booking::set_host_power_state::{
///     HostConfig,
///     PowerState,
///     set_host_power_state,
///     TimeoutConfig
///};
///use tokio::time::Duration;
///
///#[tokio::main]
///async fn main() {
///    let config = HostConfig {
///        fqdn: "example.domain.local".to_string(),
///        user: "admin".to_string(),
///        password: "password123".to_string(),
///    }
///    let timeout_config = TimeoutConfig::default
///    let result = set_host_power_state(&config, timeout_config, PowerState::On).await;
///    match result {
///        Ok(state) => println!("Power state set to: {:?}", state),
///        Err(e) => println!("Failed to set power state: {:?}", e),
///    }
///}
/// ```
///
/// # Errors
///
/// This function will return the following errors:
///
/// - [`PowerStateError::HostUnreachable`] if the host is not pingable within the timeout,
///
/// - [`PowerStateError::TimeoutReached`] if the desired state is not achieved within the timeout.
///
/// - [`PowerStateError::CommandExecutionFailed`] if the command fails to execute.
///
/// - [`PowerStateError::CommandNonZeroExitStatus`] if the command returns a non-zero exit status.
///
/// - [`PowerStateError::InvalidInputParameter`] if the input parameters are invalid.
///
/// - [`PowerStateError::Utf8Error`] if the output of the command is not valid UTF-8.
///
/// - [`PowerStateError::UnknownPowerState`] if the output of the command is not recognized.
///
/// - [`PowerStateError::SetUnknown`] if the desired state is [`PowerState::Unknown`]. This is not allowed.
///
///
pub async fn set_host_power_state(
    config: &HostConfig,
    timeout_config: TimeoutConfig,
    desired_state: PowerState,
) -> Result<PowerState, PowerStateError> {
    let power_command = match desired_state {
        PowerState::On => "on",
        PowerState::Off => "off",
        PowerState::Reset => "reset",
        PowerState::Unknown => return Err(PowerStateError::SetUnknown),
    };

    execute_power_command(config, power_command).await?;
    confirm_power_state(
        config,
        &timeout_config,
        Some(Duration::from_secs(5)),
        desired_state,
    )
    .await
}

/// Executes an IPMI power control command on a host.
///
/// # Arguments
///
/// * `config` - A reference to [`HostConfig`] containing the host's connection details.
/// * `power_command` - A string slice representing the power command (`"on"`, `"off"`, or `"reset"`).
///   technically, other commands are possible, but we shouldn't need them.
///
/// # Returns
///
/// Returns a [`Result`] of `()` if the command executes successfully and the output of the command is parsed correctly.
/// Returns [`PowerStateError`] if the command fails to execute or the host responds with a non-zero exit status.
///
/// # Errors
///
/// - [`PowerStateError::CommandExecutionFailed`] if the `ipmitool` command fails to execute.
///
/// - [`PowerStateError::CommandNonZeroExitStatus`] if the `ipmitool` command executes but returns a non-zero exit status.
///   This includes the exit code and any error message from the standard error output.
///
/// - [`PowerStateError::Utf8Error`] if the standard error output of the command is not valid UTF-8.
///
pub async fn execute_power_command(
    config: &HostConfig,
    power_command: &str,
) -> Result<(), PowerStateError> {
    let output = Command::new("ipmitool")
        .args([
            "-I",
            "lanplus",
            "-C",
            "3",
            "-H",
            &config.fqdn,
            "-U",
            &config.user,
            "-P",
            &config.password,
            "chassis",
            "power",
            power_command,
        ])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| PowerStateError::CommandExecutionFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = str::from_utf8(&output.stderr)
            .map_err(|e| PowerStateError::Utf8Error(e.to_string()))?;
        error!("IPMI command failed to execute properly");
        return Err(PowerStateError::CommandNonZeroExitStatus(
            output.status.code().expect("Expected exit code"),
            stderr.into(),
        ));
    }

    Ok(())
}

/// Confirms the power state of a host matches the desired state.
///
/// This asynchronous function repeatedly checks the current power state of the host until it matches
/// the desired state or until a timeout is reached. The check is performed a number of times as specified
/// in `timeout_config.max_retries`, waiting `timeout_config.retry_interval` between each attempt
/// for a maximum of duration of `timeout_config.timeout_duration`.
///
/// # Arguments
///
/// * `config` - A reference to [`HostConfig`] containing the host's connection details.
/// * `timeout_config` - A reference to [`TimeoutConfig`] specifying the timeout settings.
/// * `wait_for` - An optional [`Duration`] to wait before executing the command.
/// * `desired_state` - The [`PowerState`] that the host should be in.
///
/// # Returns
///
/// Returns a [`Result`] with [`PowerState`] on [`Ok`] if the current power state of the host
/// matches the desired state within the timeout, otherwise the function will return an
/// [`Err`] variant of [`PowerStateError`].
///
/// # Errors
///
/// - [`PowerStateError::TimeoutReached`] if the desired power state is not confirmed within the specified timeout and retries.
///
/// - Errors from [`get_host_power_state()`] function, including [`PowerStateError::CommandExecutionFailed`],
///   [`PowerStateError::CommandNonZeroExitStatus`], [`PowerStateError::Utf8Error`],
///   [`PowerStateError::UnknownPowerState`], etc.
///
pub async fn confirm_power_state(
    config: &HostConfig,
    timeout_config: &TimeoutConfig,
    wait_for: Option<Duration>,
    desired_state: PowerState,
) -> Result<PowerState, PowerStateError> {
    let start_time = tokio::time::Instant::now();

    if let Some(wait_for) = wait_for {
        tokio::time::sleep(wait_for).await;
    }
    for attempt in 0..timeout_config.max_retries {
        if let Some(remaining) = Duration::from_secs(timeout_config.timeout_duration as u64)
            .checked_sub(start_time.elapsed())
        {
            info!("Attempt {} to confirm power state.", attempt + 1);
            match timeout(remaining, get_host_power_state(config)).await {
                Ok(Ok(new_state)) if new_state == desired_state => {
                    info!("Desired power state confirmed.");
                    return Ok(desired_state);
                }
                Ok(Err(e)) => {
                    warn!("Error checking power state: {}", e);
                }
                Err(_) => {
                    warn!("Timeout while waiting for power state response.");
                }
                _ => {}
            }
            sleep(Duration::from_secs(timeout_config.retry_interval as u64)).await;
        } else {
            warn!("Timeout reached while waiting for power state change.");
            return Err(PowerStateError::TimeoutReached);
        }
    }

    warn!("Failed to confirm the desired power state after retries.");
    Err(PowerStateError::TimeoutReached)
}

/// Retrieves the current power state of a host using IPMI.
///
/// This asynchronous function uses [`tokio::process::Command`] and `ipmitool`to query the power
/// status of a host via IPMI. It uses the host's configuration details to
/// send the command and interprets the response to determine the current power state of the host.
///
/// # Arguments
///
/// * `config` - A reference to [`HostConfig`] containing the host's connection details.
///
/// # Returns
///
/// Returns a [`Result`] of [`PowerState`] if the command executes successfully or [`Err`] of [`PowerStateError`] otherwise.
///
/// # Examples
///
/// ```rust
/// use workflows::deploy_booking::set_host_power_state::{
///     get_host_power_state,
///     HostConfig
/// };
/// use tokio::time::Duration;
///
/// #[tokio::main]
/// async fn main() {
///     let config = HostConfig {
///         fqdn: "example.domain.local".to_string(),
///         user: "admin".to_string(),
///         password: "password123".to_string(),
///     };
///
///     match get_host_power_state(&config, Duration::from_secs(5)).await {
///         Ok(state) => println!("Current power state: {:?}", state),
///         Err(e) => println!("Failed to get power state: {:?}", e),
///     }
/// }
/// ```
///
/// # Errors
///
/// - [`PowerStateError::CommandExecutionFailed`] if the `ipmitool` command fails to execute.
///
///
/// - [`PowerStateError::CommandNonZeroExitStatus`] if the `ipmitool` command executes but returns a non-zero exit status.
///   This includes the exit code and any error message from the standard error output.
///
/// - [`PowerStateError::Utf8Error`] if the standard error or output of the command is not valid UTF-8.
///
/// - [`PowerStateError::UnknownPowerState`] if the output of the command is not recognized as a valid power state.
///
pub async fn get_host_power_state(config: &HostConfig) -> Result<PowerState, PowerStateError> {
    let output = Command::new("ipmitool")
        .args([
            "-I",
            "lanplus",
            "-C",
            "3",
            "-H",
            &config.fqdn,
            "-U",
            &config.user,
            "-P",
            &config.password,
            "chassis",
            "power",
            "status",
        ])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| PowerStateError::CommandExecutionFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = str::from_utf8(&output.stderr)
            .map_err(|e| PowerStateError::Utf8Error(e.to_string()))?;
        error!("IPMI command failed to execute properly");
        return Err(PowerStateError::CommandNonZeroExitStatus(
            output.status.code().expect("Expected exit code"),
            stderr.into(),
        ));
    }

    let output_str =
        str::from_utf8(&output.stdout).map_err(|e| PowerStateError::Utf8Error(e.to_string()))?;

    debug!("Successfully got chassis power status: {}", output_str);

    match output_str.trim() {
        "Chassis Power is on" => Ok(PowerState::On),
        "Chassis Power is off" => Ok(PowerState::Off),
        _ => {
            warn!(
                "IPMI get host power status command had unexpected output: {}",
                output_str
            );
            Err(PowerStateError::UnknownPowerState(output_str.into()))
        }
    }
}
