use std::io;
use std::path::PathBuf;
use serde::Deserialize;
use thiserror::Error;
use tracing_subscriber::{fmt, EnvFilter};
use crate::network::domain::NetworkRow;

#[derive(Debug, Deserialize)]
pub struct System {
    pub id_edge: String,
    pub host_server: String,
    pub host_local: String,
    pub db_path: String,
}


impl System {
    pub fn new(id_edge: String, host_server: String, host_local: String, db_path: String) -> Self {
        Self {
            id_edge,
            host_server,
            host_local,
            db_path,
        }
    }
}


#[derive(Error, Debug)]
pub enum ErrorType {
    #[error("❌ Error genérico")]
    MosquittoNotInstalled,

    #[error("❌ Error genérico")]
    MosquittoServiceInactive,

    #[error("❌ Error genérico")]
    Generic,

    #[error("{0}")]
    MosquittoConf(String),

    #[error("{0}")]
    MtlsConfig(String),

    #[error("{0}")]
    SystemFile(String),

    #[error("{0}")]
    NetworkFile(String),

    #[error("❌ Error de lectura/escritura (IO)")]
    Io(#[from] io::Error),

    #[error("❌ Error de lectura/escritura (IO)")]
    ClientError(#[from] rumqttc::ClientError),

    #[error("❌ Error de SQLite")]
    SQLiteError(#[from] sqlx::Error),
}


#[derive(Debug, Default)]
pub struct MtlsConfig {
    pub listener: Option<u16>,
    pub tls_version: Option<String>,
    pub broker: Certs,
    pub edge_local: Certs,
    pub edge_remote: Certs,
    pub require_certificate: bool,
    pub use_identity_as_username: bool,
    pub allow_anonymous: bool,
    pub connection_messages: bool,
}


#[derive(Debug, Default)]
pub struct Certs {
    pub cafile: PathBuf,
    pub certfile: PathBuf,
    pub keyfile: PathBuf,
}


#[derive(Debug, Deserialize)]
pub struct NetworksFile {
    pub networks: Vec<NetworkRow>,
}


pub enum StateInit {
    CheckSystem,
    ConfigSystem,
    InitSystem,
}


pub fn init_tracing() {
    let filter = EnvFilter::from_default_env()
        .add_directive("info".parse().unwrap());

    fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_level(true)
        .init();
}