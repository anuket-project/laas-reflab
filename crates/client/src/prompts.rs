use crate::remote::{Password, Select, Server, Text};
use common::prelude::inquire::validator::Validation;
use std::path::PathBuf;
use std::str::FromStr;

pub fn prompt_text(
    session: &Server,
    message: &str,
) -> Result<String, common::prelude::anyhow::Error> {
    Text::new(message).prompt(session)
}

pub fn prompt_optional_text(
    session: &Server,
    message: &str,
) -> Result<Option<String>, common::prelude::anyhow::Error> {
    match Text::new(message).prompt(session)?.as_str() {
        "" => Ok(None),
        s => Ok(Some(s.to_owned())),
    }
}

pub fn prompt_optional_path(
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

pub fn prompt_optional_password(
    session: &Server,
    message: &str,
) -> Result<Option<String>, common::prelude::anyhow::Error> {
    match Password::new(message).prompt(session)?.as_str() {
        "" => Ok(None),
        s => Ok(Some(s.to_owned())),
    }
}

pub fn prompt_boolean(
    session: &Server,
    message: &str,
) -> Result<Option<bool>, common::prelude::anyhow::Error> {
    match Select::new(message, vec!["true", "false"]).prompt(session)? {
        "false" => Ok(None),
        "true" => Ok(Some(true)),
        _ => Ok(None),
    }
}

pub fn prompt_optional_comma_separated(
    session: &Server,
    message: &str,
) -> Result<Option<Vec<String>>, common::prelude::anyhow::Error> {
    match Text::new(message).prompt(session)?.as_str() {
        "" => Ok(None),
        s => Ok(Some(s.split(',').map(|s| s.to_owned()).collect())),
    }
}

pub fn prompt_user_auth_type(
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
