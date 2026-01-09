//! Definiciones de configuración y gestión de errores.
//!
//! Este módulo actúa como el núcleo de configuración de la aplicación. Define:
//! 1. **Identidad del Edge:** Quién es el dispositivo y dónde están sus recursos clave (`System`).
//! 2. **Tipos de Error:** Unificación de errores de IO, Base de Datos y Lógica de Negocio (`ErrorType`).
//! 3. **Configuración de Seguridad:** Estructuras para manejar certificados y configuración mTLS (`MtlsConfig`).
//! 4. **Inicialización:** Estados de arranque y configuración de logging (`init_tracing`).


use std::io;
use std::path::PathBuf;
use serde::Deserialize;
use thiserror::Error;
use tracing_subscriber::{fmt, EnvFilter};
use crate::network::domain::NetworkRow;


/// Representación inmutable de la identidad y configuración base del dispositivo Edge.
///
/// Contiene la información estática necesaria para que el sistema sepa quién es,
/// con quién debe hablar y dónde guardar sus datos.
#[derive(Debug, Deserialize)]
pub struct System {
    /// Identificador único del dispositivo (ej. `edge-001`, `sala-maquinas`).
    pub id_edge: String,

    /// Dirección (IP o Hostname) del servidor central.
    pub host_server: String,

    /// Dirección (IP o Hostname) local del dispositivo.
    pub host_local: String,

    /// Ruta relativa al archivo de base de datos SQLite (ej. `./data/edge.db`).
    pub db_path: String,
}


impl System {
    /// Crea una nueva instancia de la configuración del sistema.
    pub fn new(id_edge: String, host_server: String, host_local: String, db_path: String) -> Self {
        Self {
            id_edge,
            host_server,
            host_local,
            db_path,
        }
    }
}


/// Enumeración centralizada de todos los posibles errores del sistema.
///
/// Utiliza el crate `thiserror` para simplificar la definición y conversión
/// de errores externos (IO, SQLx, Rumqttc) en un tipo propio del dominio.
#[derive(Error, Debug)]
pub enum ErrorType {
    #[error("Error mosquitto no instalado")]
    MosquittoNotInstalled,

    #[error("Error servicio inactivo")]
    MosquittoServiceInactive,

    #[error("Error genérico")]
    Generic,

    #[error("{0}")]
    MosquittoConf(String),

    #[error("{0}")]
    MtlsConfig(String),

    #[error("{0}")]
    SystemFile(String),

    #[error("{0}")]
    NetworkFile(String),

    #[error("Error de lectura/escritura (IO)")]
    Io(#[from] io::Error),

    #[error("Error de cliente mqtt")]
    ClientError(#[from] rumqttc::ClientError),

    #[error("Error de SQLite")]
    SQLiteError(#[from] sqlx::Error),
}


/// Configuración para la seguridad de transporte (mTLS) del Broker.
///
/// Esta estructura se utiliza para generar el archivo de configuración
/// de Mosquitto, definiendo puertos, versiones de TLS y ubicaciones de certificados.
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


/// Agrupación de rutas a los archivos de certificados X.509.
#[derive(Debug, Default)]
pub struct Certs {
    pub cafile: PathBuf,
    pub certfile: PathBuf,
    pub keyfile: PathBuf,
}


/// Estructura DTO (Data Transfer Object) para la carga inicial de redes.
///
/// Representa el contenido deserializado de un archivo de configuración (toml)
/// que contiene la lista de redes predefinidas.
#[derive(Debug, Deserialize)]
pub struct NetworksFile {
    pub networks: Vec<NetworkRow>,
}


/// Estados del proceso de arranque (Bootstrapping) del sistema.
pub enum StateInit {
    CheckSystem,
    ConfigSystem,
    InitSystem,
}


/// Inicializa el sistema de logging y trazas (Tracing).
///
/// Configura un suscriptor de `tracing` que imprime en la salida estándar (stdout).
///
/// # Configuración
/// - **Nivel por defecto:** `INFO` (se puede sobreescribir con variable de entorno `RUST_LOG`).
/// - **Target:** Desactivado (no muestra la ruta del módulo para limpiar la salida).
/// - **Nivel:** Activado (muestra si es INFO, WARN, ERROR).
pub fn init_tracing() {
    let filter = EnvFilter::from_default_env()
        .add_directive("info".parse().unwrap());

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_level(true)
        .init();
}