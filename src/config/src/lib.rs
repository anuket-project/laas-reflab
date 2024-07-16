// Copyright (c) 2023 University of New Hampshire
// SPDX-License-Identifier: MIT
#![doc = include_str!("../README.md")]
//! # Sample Config
//! ```yaml
#![doc = include_str!("../../../sample_config.yaml")]
//! ```

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use strum_macros::Display;
use tracing_subscriber::filter::LevelFilter;

#[derive(Debug, Deserialize, Clone)]
pub struct LibLaaSConfig {
    pub dev: Dev,
    pub database: DatabaseConfig,
    pub web: WebConfig,
    pub mailbox: MailboxConfig,
    pub cli: CliConfig,
    pub notifications: NotificationConfig,
    pub cobbler: CobblerConfig,
    pub ipa: Vec<IPAConfig>,
    pub projects: HashMap<String, ProjectConfig>,
    #[serde(default)]
    pub metrics: Option<MetricsConfig>,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum LoggingLevel {
    ERROR,
    WARN,
    #[default]
    INFO,
    DEBUG,
    TRACE,
    OFF,
}

impl<'de> Deserialize<'de> for LoggingLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;

        Ok(match v.as_str() {
            "ERROR" => Self::ERROR,
            "WARN" => Self::WARN,
            "INFO" => Self::INFO,
            "DEBUG" => Self::DEBUG,
            "TRACE" => Self::TRACE,
            "OFF" => Self::OFF,
            other => Err(serde::de::Error::custom(format!(
                "Bad situation specifier {other}"
            )))?,
        })
    }
}

impl From<LoggingLevel> for LevelFilter {
    fn from(value: LoggingLevel) -> Self {
        match value {
            LoggingLevel::ERROR => LevelFilter::ERROR,
            LoggingLevel::WARN => LevelFilter::WARN,
            LoggingLevel::INFO => LevelFilter::INFO,
            LoggingLevel::DEBUG => LevelFilter::DEBUG,
            LoggingLevel::TRACE => LevelFilter::TRACE,
            LoggingLevel::OFF => LevelFilter::OFF,
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub log_file: Option<String>,

    #[serde(default)]
    pub max_level: LoggingLevel,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Email {
    pub username: String,
    pub domain: String,
}

impl Email {
    pub fn as_address_string(&self) -> String {
        format!("{}@{}", self.username, self.domain)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Dev {
    pub status: bool,
    pub hosts: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: HostPortPair,
    pub username: String,
    pub password: String,
    pub database_name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WebConfig {
    pub bind_addr: HostPortPair,
    pub external_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MailboxConfig {
    pub bind_addr: HostPortPair,
    pub external_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CliConfig {
    pub bind_addr: HostPortPair,
    pub external_url: HostPortPair,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NotificationConfig {
    pub mail_server: HostPortPair,

    pub send_from_email: Option<Email>,

    /// If present, send any critical "admin" errors to this gchat
    pub admin_gchat_webhook: Option<String>,

    /// If present, send any critical "admin" errors to this mail
    pub admin_mail_server: Option<HostPortPair>,

    pub admin_send_from_email: Option<Email>,

    pub admin_send_to_email: Option<Email>,
    pub templates_directory: String
}
#[derive(Debug, Deserialize, Clone)]
pub struct CobblerConfig {
    pub url: String,
    pub username: String,
    pub password: String,
}
#[derive(Debug, Deserialize, Clone)]
pub struct IPAConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    pub certificate_path: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MetricsConfig {
    pub max_failover: u8,
    pub client_retries: u8,
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProjectConfig {
    pub vpn: VPNConfig,
    pub notifications: HashMap<RenderTarget, HashMap<Situation, String>>,
    pub styles_path: String,
    pub dashboard_url: String,
    pub search_domains: Vec<String>,
    pub nameservers: Vec<String>,
    pub location: String,
    pub email: String,
    pub phone: String,
    pub is_dynamic: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VPNConfig {
    pub ipa_group: String,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub enum RenderTarget {
    Email,
}

impl<'de> Deserialize<'de> for RenderTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;

        Ok(match v.as_str() {
            "email" => Self::Email,
            other => Err(serde::de::Error::custom(format!(
                "Bad rendertarget specifier {other}"
            )))?,
        })
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Display)]
pub enum Situation {
    BookingCreated,
    BookingExpiring,
    BookingExpired,
    VPNAccessAdded,
    VPNAccessRemoved,
    PasswordResetRequested,
    AccountCreated,
    CollaboratorAdded(Vec<String>),
    RequestBookingExtension,
}

impl<'de> Deserialize<'de> for Situation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;

        Ok(match v.as_str() {
            "booking_created" => Self::BookingCreated,
            "booking_expiring" => Self::BookingExpiring,
            "booking_expired" => Self::BookingExpired,
            "vpn_access_added" => Self::VPNAccessAdded,
            "vpn_access_removed" => Self::VPNAccessRemoved,
            "account_created" => Self::AccountCreated,
            "collaborator_added" => Self::CollaboratorAdded(Vec::new()),
            "booking_extension_request" => Self::RequestBookingExtension,
            other => Err(serde::de::Error::custom(format!(
                "Bad situation specifier {other}"
            )))?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct HostPortPair {
    pub host: String,
    pub port: u16,
}

impl<'de> Deserialize<'de> for HostPortPair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let base = String::deserialize(deserializer)?;

        let (host, port) = base
            .split_once(':')
            .ok_or(serde::de::Error::custom(format!(
                "Failed to split {base} into component host and port"
            )))?;

        let port = port.parse().map_err(|_e| {
            serde::de::Error::custom(format!("Couldn't parse out port as an int from {port}"))
        })?;

        Ok(HostPortPair {
            host: host.to_owned(),
            port,
        })
    }
}

impl ToString for HostPortPair {
    fn to_string(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

static CONFIG: once_cell::sync::Lazy<LibLaaSConfig> = once_cell::sync::Lazy::new(|| {
    config::Config::builder()
        .add_source(config::File::with_name("config_data/config.yaml"))
        .build()
        .expect("couldn't load config file")
        .try_deserialize()
        .expect("couldn't load config file, invalid format")
});

pub fn settings() -> &'static LibLaaSConfig {
    &CONFIG
}
