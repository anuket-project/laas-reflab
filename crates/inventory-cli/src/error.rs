use thiserror::Error;

#[derive(Debug)]
pub struct MultipleErrors(pub Vec<InventoryError>);

impl std::fmt::Display for MultipleErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for error in &self.0 {
            writeln!(f, "   {}", error)?;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("duplicate host in DB with server_name `{0}`")]
    DuplicateHost(String),

    #[error("{0}")]
    Env(#[from] std::env::VarError),

    #[error("parsing YAML `{path}`: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("{0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Anyhow(#[from] anyhow::Error),

    #[error("SQLX error: {context}: {source}")]
    Sqlx {
        context: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("{0}")]
    Glob(#[from] glob::GlobError),

    #[error("{0}")]
    Pattern(#[from] glob::PatternError),

    #[error("reading `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid MAC `{raw}` for server: {server_name}: {source}")]
    InvalidMac {
        server_name: String,
        raw: String,
        #[source]
        source: eui48::ParseError,
    },

    #[error("{0}")]
    NotFound(String),

    #[error("Invalid inventory path `{path}`: {message}")]
    IoPath {
        path: std::path::PathBuf,
        message: String,
    },

    #[error("Invalid FQDN `{fqdn}`: {msg}")]
    InvalidFQDN { fqdn: String, msg: String },

    #[error("Wrong host provided. Expected `{expected}` but got `{actual}`")]
    HostNameMismatch { expected: String, actual: String },

    #[error("Called `record` on a non-modified report")]
    RecordOnNonModifiedReport,

    #[error("Field `{0}` already modified")]
    FieldAlreadyModified(String),

    #[error("Host has too many projects: `{0:?}`")]
    TooManyProjects(Vec<String>),

    #[error("Error(s) encountered while attempting to parse inventory files: \n {0} ")]
    InventoryErrors(MultipleErrors),

    #[error("Invalid project(s) on `{server_name}`: {source}")]
    InvalidProjects {
        server_name: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("Aborted")]
    Aborted,

    #[error("{0}")]
    Conflict(String),

    #[error("Error encountered while parsing address: {value}: {source}")]
    AddrParse {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },

    #[error("Invalid report type: {expected}, got {actual}")]
    InvalidReportType {
        expected: &'static str,
        actual: &'static str,
    },

    #[error("{0}")]
    NotImplemented(String),
}
