//! Configuración básica del entorno Mosquitto con soporte mTLS.
//!
//! Este módulo se encarga exclusivamente de:
//!
//! - Habilitar y reiniciar el servicio `mosquitto`.
//! - Crear archivos de configuración básicos (`mosquitto.conf` y `mtls.conf`).
//! - Establecer permisos y ownership seguros sobre dichos archivos.
//!
//! ## Alcance
//!
//! - **No genera certificados**
//! - **No valida certificados**
//! - **No gestiona claves privadas**
//!
//! Los certificados deben existir previamente en las rutas documentadas.
//!
//! ## Filosofía
//!
//! - Configuración mínima y explícita
//! - Sin valores implícitos
//!
//! ## Seguridad
//!
//! - Todos los archivos son propiedad de `root`.
//! - Permisos Unix conservadores (`0644`).
//! - No se escriben secretos desde el binario.
//!


use std::fs;
use std::fs::{File};
use std::io::{Write};
use std::process::Command;
use std::ffi::CString;
use libc::{chown, uid_t, gid_t};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path};
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::system::domain::{ErrorType, Flag, System};
use tracing::{info, error, instrument};
use crate::config::files::{PROTOCOL_TOML_PATH, SYSTEM_TOML_PATH};
use crate::context::domain::AppContext;
use crate::network::domain::{NetworkManager};
use crate::quorum::domain::ProtocolSettings;


/// Configura el sistema en función de un evento de la FSM.
///
/// Actúa como punto de entrada del módulo y decide qué acciones
/// ejecutar según el tipo de inconsistencia detectada.
///
/// # Parámetros
///
/// - `event`: bandera de la FSM que indica qué componente del sistema
///   requiere configuración.
///
/// # Comportamiento
///
/// Dependiendo del evento:
///
/// - Habilita el servicio `mosquitto`.
/// - Crea o recrea archivos de configuración.
/// - Reinicia el servicio para aplicar cambios.
///

#[instrument(name = "configurate_system")]
pub fn configurate_system(event: &Flag) -> Result<(), ErrorType> {

    match event {
        Flag::MosquittoServiceInactive => {
            mosquitto_service_inactive_flow()?;
        },
        Flag::MosquittoConf => {
            mosquitto_conf_flow()?;
        },
        Flag::MtlsConf => {
            mtls_conf_flow()?;
        },
        _ => {},
    }
    
    Ok(())
}


fn mosquitto_service_inactive_flow() -> Result<(), ErrorType> {
    match activate_service() {
        Ok(_) => info!("Éxito: servicio mosquitto activado correctamente"),
        Err(e) => {
            error!("Error: systemctl falló al activar mosquitto. {e}");
            return Err(e);
        }
    }

    match create_mosquitto_conf() {
        Ok(_) => info!("Éxito: archivo de configuración de mosquitto creado correctamente"),
        Err(e) => {
            error!("Error: no se pudo crear el archivo de configuración de mosquitto. {e}");
            return Err(e);
        }
    }

    match create_mtls_config_file() {
        Ok(_) => info!("Éxito: archivo de configuración de mTLS creado correctamente"),
        Err(e) => {
            error!("Error: no se pudo crear el archivo de configuración de mTLS. {e}");
            return Err(e);
        }
    }

    match restart_service() {
        Ok(_) => info!("Éxito: servicio mosquitto reiniciado correctamente"),
        Err(e) => {
            error!("Error: systemctl falló al reiniciar mosquitto. {e}");
            return Err(e);
        }
    }
    Ok(())
}


fn mosquitto_conf_flow() -> Result<(), ErrorType> {
    match create_mosquitto_conf() {
        Ok(_) => info!("Éxito: archivo de configuración de mosquitto creado correctamente"),
        Err(e) => {
            error!("Error: no se pudo crear el archivo de configuración de mosquitto. {e}");
            return Err(e);
        }
    }

    match create_mtls_config_file() {
        Ok(_) => info!("Éxito: archivo de configuración de mTLS creado correctamente"),
        Err(e) => {
            error!("Error: no se pudo crear el archivo de configuración de mTLS. {e}");
            return Err(e);
        }
    }

    match restart_service() {
        Ok(_) => info!("Éxito: servicio mosquitto reiniciado correctamente"),
        Err(e) => {
            error!("Error: systemctl falló al reiniciar mosquitto. {e}");
            return Err(e);
        }
    }
    Ok(())
}


fn mtls_conf_flow() -> Result<(), ErrorType> {
    match create_mtls_config_file() {
        Ok(_) => info!("Éxito: archivo de configuración de mTLS creado correctamente"),
        Err(e) => {
            error!("Error: no se pudo crear el archivo de configuración de mTLS. {e}");
            return Err(e);
        }
    }

    match restart_service() {
        Ok(_) => info!("Éxito: servicio mosquitto reiniciado correctamente"),
        Err(e) => {
            error!("Error: systemctl falló al reiniciar mosquitto. {e}");
            return Err(e);
        }
    }
    Ok(())
}


/// Habilita e inicia el servicio `mosquitto` usando systemd.
///
/// Ejecuta:
///
/// ```text
/// systemctl enable --now mosquitto
/// ```
///
/// # Retorno
///
/// - `Ok(())` si el comando finaliza con exit code 0.
/// - `Err(ErrorType::Generic)` si el comando falla o no puede ejecutarse.
///
/// # Requisitos
///
/// - Sistema Linux con systemd.
/// - Permisos de superusuario.

fn activate_service() -> Result<(), ErrorType> {
    // Usamos "enable --now" que hace dos cosas: lo habilita para que arranque al inicio (boot) y lo inicia inmediatamente
    let status = Command::new("systemctl")
        .arg("enable")
        .arg("--now")
        .arg("mosquitto")
        .status() // Solo nos interesa si funciono o no (el exit code)
        .map_err(|_| ErrorType::Generic)?;  // Error si no se pudo ejecutar el comando

    if status.success() {
        Ok(())  // El comando devolvio exit code 0
    } else {
        Err(ErrorType::Generic)
    }
}


/// Crea el archivo principal de configuración `mosquitto.conf`.
///
/// Este archivo define:
///
/// - Persistencia de mensajes.
/// - Ubicación de logs.
/// - Inclusión del directorio `conf.d` para configuraciones adicionales.
///
/// # Comportamiento
///
/// - Crea el directorio `/etc/mosquitto` si no existe.
/// - Sobrescribe el archivo si ya existe.
/// - Establece permisos `0644`.
/// - Asigna ownership a `root`.
///
/// # Retorno
///
/// - `Ok(())` si el archivo se crea correctamente.
/// - `Err(ErrorType::Generic)` ante cualquier fallo de IO o permisos.
///

fn create_mosquitto_conf() -> Result<(), ErrorType> {
    let path = "/etc/mosquitto";
    let path_file = "/etc/mosquitto/mosquitto.conf";

    if let Err(_) = fs::create_dir_all(path) {
        return Err(ErrorType::Generic);
    }

    // File::create crea o vacia el archivo si ya existe
    let mut file = File::create(path_file)?;

    // Usamos r#""# porque permite texto multilinea
    let text = r#"# Configuracion basica de mosquitto

# Guarda informacion critica en disco
persistence true

# En el siguiente path se encuentra la base de datos con la informacion de persistencia
persistence_location /var/lib/mosquitto/

# Define donde van los mensajes de diagnostico y errores
log_dest file /var/log/mosquitto/mosquitto.log

# La configuracion de mtls se encuentra en /etc/mosquitto/conf.d/
include_dir /etc/mosquitto/conf.d
"#;

    file.write_all(text.as_bytes())?;

    fs::set_permissions(path_file, fs::Permissions::from_mode(0o644))
        .map_err(|_| ErrorType::Generic)?;

    chown_root(path_file)?;
    Ok(())
}


/// Crea el archivo `mtls.conf` con la configuración TLS del broker.
///
/// Define:
///
/// - Puerto TLS.
/// - Versión de TLS permitida.
/// - Rutas a certificados del broker y edges.
/// - Reglas estrictas de autenticación mutua.
///
/// # Importante
///
/// - Este archivo **no genera certificados**.
/// - Las rutas deben existir previamente.
/// - El contenido es estático y documenta el layout esperado.
///
/// # Comportamiento
///
/// - Crea `/etc/mosquitto/conf.d` si no existe.
/// - Sobrescribe el archivo si ya existe.
/// - Establece permisos `0644`.
/// - Asigna ownership a `root`.
///
/// # Retorno
///
/// - `Ok(())` si el archivo se crea correctamente.
/// - `Err(ErrorType::Generic)` ante cualquier fallo.
///
/// # Seguridad
///
/// El archivo referencia claves privadas, pero no las contiene.

fn create_mtls_config_file() -> Result<(), ErrorType> {
    let path = "/etc/mosquitto/conf.d";
    let path_file = "/etc/mosquitto/conf.d/mtls.conf";

    if let Err(_) = fs::create_dir_all(path) {
        return Err(ErrorType::Generic);
    }

    // File::create crea o vacia el archivo si ya existe
    let mut file = File::create(path_file)?;

    // Usamos r#""# porque permite texto multilinea
    let text = r#"# Configuracion para IoT Edge Gateway
listener 8883
tls_version tlsv1.2
cafile /etc/mosquitto/certs/root.crt
certfile /etc/mosquitto/certs/edge.crt
keyfile /etc/mosquitto/certs/edge.key
require_certificate true
use_identity_as_username true
allow_anonymous false
connection_messages true
"#;

    file.write_all(text.as_bytes())?;

    fs::set_permissions(path_file, fs::Permissions::from_mode(0o644))
        .map_err(|_| ErrorType::Generic)?;

    chown_root(path_file)?;
    Ok(())
}


/// Reinicia el servicio `mosquitto`.
///
/// Ejecuta:
///
/// ```text
/// systemctl restart mosquitto
/// ```
///
/// # Retorno
///
/// - `Ok(())` si el reinicio fue exitoso.
/// - `Err(ErrorType::Generic)` si el comando falla.
///

fn restart_service() -> Result<(), ErrorType> {
    let status = Command::new("systemctl")
        .arg("restart")
        .arg("mosquitto")
        .status() // Solo nos interesa si funciono o no (el exit code)
        .map_err(|_| ErrorType::Generic)?;  // Error si no se pudo ejecutar el comando

    if status.success() {
        Ok(())  // El comando devolvio exit code 0
    } else {
        Err(ErrorType::Generic)
    }
}


/// Asigna ownership del archivo a `root:root`.
///
/// Utiliza la syscall `chown` directamente a través de `libc`.
///
/// # Parámetros
///
/// - `path`: ruta al archivo.
///
/// # Retorno
///
/// - `Ok(())` si el ownership se aplica correctamente.
/// - `Err(ErrorType::Generic)` si la syscall falla.
///
/// # Seguridad
///
/// Requiere privilegios de superusuario.
/// Se utiliza para evitar archivos controlados por usuarios no privilegiados.

fn chown_root(path: &str) -> Result<(), ErrorType> {
    let c_path = CString::new(path).unwrap();
    let res = unsafe { chown(c_path.as_ptr(), 0 as uid_t, 0 as gid_t) };

    if res == 0 {
        Ok(())
    } else {
        Err(ErrorType::Generic)
    }
}


/// Inicializa los componentes fundamentales del sistema y crea el `AppContext`
///
/// Lee los datos del archivo `system.toml` (el cual persiste y es de solo lectura) y
/// crea la estructura `System` con los campos fundamentales de funcionamiento del sistema.
/// Luego sigue con la lectura del archivo `protocol.toml`, el cual contiene la información necesaria
/// para configurar el comportamiento del protocolo de control y balanceo post fallo.
///
/// # Retorno
///
/// - `AppContext` si finaliza con éxito.
/// - `ErrorType` si falla o no puede ejecutarse.
///
/// # Requisitos del file system
///
/// etc/edge/files/system.toml
/// etc/edge/files/protocol.toml
///

pub fn initializing_system() -> Result<AppContext, ErrorType> {

    let system = match load_system_toml(Path::new(SYSTEM_TOML_PATH)) {
        Ok(system) => system,
        Err(e) => return Err(e),
    };
    let system = Arc::new(system);

    let protocol = match load_protocol_toml(Path::new(PROTOCOL_TOML_PATH)) {
        Ok(system) => system,
        Err(e) => return Err(e),
    };
    let protocol = Arc::new(protocol);
    let net_man = Arc::new(RwLock::new(NetworkManager::new_empty(&system)));
    Ok(AppContext::new(net_man, system, protocol))
}


/// Carga los datos del archivo `system.toml`
///
/// Lee los datos del archivo `system.toml`, el cual tiene los campos que necesita
/// la estructura `System` para funcionar.
///
/// # Retorno
///
/// - `System` si finaliza con éxito.
/// - `ErrorType` si falla o no puede ejecutarse.
///
/// # Requisitos del file system
///
/// El archivo toml en: `etc/edge/files/system.toml`
///

fn load_system_toml(path: &Path) -> Result<System, ErrorType> {
    let content = fs::read_to_string(path).map_err(|e| {
        ErrorType::SystemFile(format!(
            "Error al leer la configuración en '{}': {}",
            path.display(),
            e
        ))
    })?;

    toml::from_str(&content).map_err(|e| {
        ErrorType::SystemFile(format!(
            "Error de sintaxis en el TOML de configuración '{}': {}",
            path.display(),
            e
        ))
    })
}


/// Carga los datos del archivo `protocol.toml`
///
/// Lee los datos del archivo `protocol.toml`, el cual tiene los campos que necesita
/// la estructura `ProtocolSettings` para funcionar.
///
/// # Retorno
///
/// - `ProtocolSettings` si finaliza con éxito.
/// - `ErrorType` si falla o no puede ejecutarse.
///
/// # Requisitos del file system
///
/// El archivo toml en: `etc/edge/files/protocol.toml`
///

fn load_protocol_toml(path: &Path) -> Result<ProtocolSettings, ErrorType> {

    let content = fs::read_to_string(path).map_err(|e| {
        ErrorType::ProtocolFile(format!(
            "Error al leer la configuración en '{}': {}",
            path.display(),
            e
        ))
    })?;

    toml::from_str(&content).map_err(|e| {
        ErrorType::ProtocolFile(format!(
            "Error de sintaxis en el TOML de configuración '{}': {}",
            path.display(),
            e
        ))
    })
}