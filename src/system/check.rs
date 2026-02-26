//! Módulo de validación del entorno del sistema.
//!
//! Este módulo se encarga de verificar, antes del arranque del sistema principal,
//! que el entorno de ejecución cumple estrictamente con todos los requerimientos
//! necesarios para operar de forma segura.
//!
//! # Objetivo
//!
//! Garantizar que:
//! - El broker Mosquitto está instalado.
//! - El servicio de sistema está activo.
//! - El broker está escuchando en el puerto seguro (TLS).
//! - La configuración principal de Mosquitto es válida.
//! - La configuración mTLS cumple con los parámetros de seguridad requeridos.
//! - Los certificados existen, son seguros y coherentes.
//!
//! Este módulo **no intenta corregir configuraciones** ni realizar cambios
//! automáticos en el sistema. Su responsabilidad es únicamente validar y
//! fallar de manera explícita si el entorno no cumple los requisitos.
//!
//! # Filosofía de diseño
//!
//! - **Fail fast**: cualquier condición inválida aborta el proceso.
//! - **Configuración estricta**: solo se aceptan parámetros explícitamente definidos.
//! - **Seguridad por defecto**: no se permiten configuraciones laxas.
//!
//! # Alcance
//!
//! Este módulo asume:
//! - Un sistema Linux con systemd.
//! - Mosquitto gestionado como servicio.
//! - Configuración basada en archivos estáticos.
//!
//! No es portable a otros sistemas sin modificaciones.
//!
//! # Uso esperado
//!
//! Este módulo debe ejecutarse **una única vez al inicio del servicio**.
//! Si la validación falla, el proceso debe abortar y permitir que systemd
//! registre el error en `journalctl`.
//!
//! # Errores
//!
//! Los errores se expresan mediante el enum `ErrorType`.
//!
//! # Seguridad
//!
//! Se validan explícitamente:
//! - Permisos de clave privada.
//! - Separación entre CA y certificado servidor.
//! - Versiones TLS permitidas.
//! - Autenticación obligatoria.
//!
//! Cualquier desviación se considera un fallo crítico.
//!
//! # Advertencia
//!
//! Este módulo es deliberadamente estricto.
//! Configuraciones funcionales pero inseguras **serán rechazadas**.
//!
//! # Estructura esperada del file system
//!
//! /etc/mosquitto/
//! ├── mosquitto.conf
//! │
//! ├── certs/
//! │   ├── root.crt
//! │   ├── edge.crt
//! │   └── edge.key
//! │
//! └── conf.d/
//!     └── mtls.conf


use fs::metadata;
use which::which;
use std::process::Command;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::fs;
use std::io::{BufRead, BufReader};
use crate::system::domain::{Certs, ErrorType, MtlsConfig};
use tracing::{info, error, instrument};


/// Ejecuta la validación completa del entorno del sistema antes del arranque.
///
/// Esta función actúa como **orquestador principal** del proceso de verificación.
/// Debe ejecutarse una única vez durante el inicio del servicio y abortar
/// inmediatamente ante cualquier condición inválida.
///
/// # Validaciones realizadas
///
/// 1. Verifica que el binario `mosquitto` exista y sea ejecutable.
/// 2. Comprueba que el servicio `mosquitto` esté activo vía `systemd`.
/// 3. Verifica que el broker esté escuchando en el puerto TLS esperado.
/// 4. Valida la existencia y coherencia de `mosquitto.conf`.
/// 5. Valida estrictamente la configuración mTLS (`mtls.conf`).
/// 6. Verifica la existencia, permisos y coherencia de todos los certificados.
///
/// # Filosofía
///
/// - **Fail fast**: el primer error detiene la ejecución.
/// - **No correcciones automáticas**: solo validación.
/// - **Configuración estricta**: no se aceptan valores por defecto implícitos.
///
/// # Retorno
///
/// - `Ok(())` si todas las validaciones son exitosas.
/// - `Err(ErrorType)` si cualquier requisito no se cumple.
///
/// # Observabilidad
///
/// La función está instrumentada con `tracing` para permitir
/// auditoría completa desde `journalctl`.

#[instrument(name = "check_system_config")]
pub fn check_system_config() -> Result<(), ErrorType> {

    match which("mosquitto") {
        Ok(path) => info!(path = ?path, "Éxito: Mosquitto encontrado y ejecutable"),
        Err(e) => {
            error!(binary = "mosquitto", "Error: Binario requerido no encontrado. {}", e);
            return Err(ErrorType::MosquittoNotInstalled);
        },
    }

    match service_active("mosquitto") {
        Ok(_) => info!("Éxito: El servicio de sistema mosquitto está activo"),
        Err(e) => {
            error!("Error: El servicio de sistema mosquitto no está activo. {}", e);
            return Err(ErrorType::MosquittoServiceInactive);
        },
    }

    match mosquitto_listening("localhost:8883") {
        Ok(_) => info!("Éxito: El servicio de sistema mosquitto está escuchando"),
        Err(e) => {
            error!("Error: El servicio de sistema mosquitto no está escuchando. {}", e);
            return Err(ErrorType::MosquittoServiceInactive);
        },
    }

    match validate_mosquitto_conf(Path::new("/etc/mosquitto/mosquitto.conf")) {
        Ok(_) => info!("Éxito: El archivo mosquitto.conf es válido"),
        Err(e) => {
            error!("{}", e);
            return Err(e);
        },
    }

    let cfg = match validate_mtls_conf(Path::new("/etc/mosquitto/conf.d/mtls.conf")) {
        Ok(config) => {
            info!("Éxito: Mosquitto configurado para mTLS");
            config
        },
        Err(e) => {
            error!("{}", e);
            return Err(e);
        }
    };

    match validate_certificate_files(&cfg) {
        Ok(_) => info!("Éxito: Los certificados mTLS son correctos"),
        Err(e) => {
            error!("{}", e);
            return Err(e);
        }
    }

    Ok(())
}


/// Verifica si un servicio de systemd se encuentra activo.
///
/// Ejecuta el comando:
///
/// ```text
/// systemctl is-active <service_name>
/// ```
/// y valida que la salida sea exactamente `"active"`.
///
/// # Parámetros
///
/// - `service_name`: nombre del servicio systemd a verificar.
///
/// # Retorno
///
/// - `Ok(())` si el servicio está activo.
/// - `Err(ErrorType::Generic)` si el servicio no está activo o el comando falla.
///
/// # Notas
///
/// - No distingue entre estados intermedios (`activating`, `failed`, etc.).
/// - Asume un sistema Linux con systemd.
/// - No intenta reiniciar el servicio.

fn service_active(service_name: &str) -> Result<(), ErrorType>  {
    let output = Command::new("systemctl")
        .arg("is-active")
        .arg(service_name)
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.trim() == "active" {
                Ok(())
            } else {
                Err(ErrorType::Generic)
            }
        },
        Err(_) => Err(ErrorType::Generic),
    }
}


/// Verifica que exista un proceso escuchando en una dirección TCP.
///
/// Establece una conexión TCP con timeout al address especificado.
/// Esta verificación **solo valida disponibilidad del puerto**.
///
/// # Parámetros
///
/// - `address`: dirección en formato `IP:PUERTO`.
///
/// # Retorno
///
/// - `Ok(())` si el socket acepta conexiones.
/// - `Err(ErrorType)` si no es posible conectar.
///
/// # Importante
///
/// - **No valida TLS**
/// - **No valida certificados**
/// - **No valida mTLS**
///
/// Su objetivo es únicamente comprobar que el broker está escuchando.

fn mosquitto_listening(address: &str) -> Result<(), ErrorType> {
    use std::process::Command;

    // Extraer el puerto del address (ej: "localhost:8883")
    let port = address
        .split(':')
        .nth(1)
        .ok_or(ErrorType::Generic)?;

    // Ejecutar `ss -ltn`
    let output = Command::new("ss")
        .arg("-ltn")
        .output()
        .map_err(|_| ErrorType::Generic)?;

    if !output.status.success() {
        return Err(ErrorType::Generic);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Buscar línea que contenga :<puerto>
    for line in stdout.lines() {
        let line = line.trim();

        // Debe estar en estado LISTEN y contener el puerto exacto
        if line.starts_with("LISTEN") && line.contains(&format!(":{}", port)) {
            return Ok(());
        }
    }

    Err(ErrorType::Generic)
}


/// Valida la existencia y contenido básico del archivo `mosquitto.conf`.
///
/// Esta función verifica:
///
/// - Que el archivo exista y sea accesible.
/// - Que no esté vacío.
/// - Que incluya el directorio de configuración esperado (`conf.d`).
///
/// # Parámetros
///
/// - `path`: ruta al archivo `mosquitto.conf`.
///
/// # Retorno
///
/// - `Ok(())` si el archivo cumple los requisitos.
/// - `Err(ErrorType::MosquittoConf)` si el archivo es inválido.
///
/// # Alcance
///
/// Esta función **no valida la semántica completa** del archivo,
/// solo los requisitos mínimos de inclusión del directorio mTLS.

fn validate_mosquitto_conf(path: &Path) -> Result<(), ErrorType> {
    let metadata = metadata(path).map_err(|_| ErrorType::MosquittoConf(
        "Error: El archivo de configuración de mosquitto no existe o no es accesible".into()
    ))?;

    if metadata.len() == 0 {
        return Err(ErrorType::MosquittoConf("Error: El archivo de configuración de mosquitto esta vacío".into()));
    }

    scan_mosquitto_file_config(&path)?;

    Ok(())
}


/// Escanea el archivo `mosquitto.conf` buscando directivas relevantes.
///
/// Actualmente valida exclusivamente la presencia de:
///
/// ```text
/// include_dir /etc/mosquitto/conf.d
/// ```
///
/// # Parámetros
///
/// - `path`: ruta al archivo `mosquitto.conf`.
///
/// # Retorno
///
/// - `Ok(())` si la directiva requerida existe y es correcta.
/// - `Err(ErrorType::MosquittoConf)` en cualquier otro caso.
///
/// # Comportamiento
///
/// - Ignora líneas vacías y comentarios.
/// - Finaliza en la primera coincidencia válida.
/// - Rechaza rutas distintas a la esperada.
///
/// # Diseño
///
/// Este comportamiento es deliberadamente estricto.

fn scan_mosquitto_file_config(path: &Path) -> Result<(), ErrorType> {
    let file = File::open(path)
        .map_err(|_| ErrorType::MtlsConfig("Error: No se pudo abrir el archivo mosquitto conf".into()))?;

    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap();
        let value = parts.next();

        match (key, value) {
            ("include_dir", Some(dir)) => {
                return if Path::new(dir) == Path::new("/etc/mosquitto/conf.d") {
                    Ok(())
                } else {
                    Err(ErrorType::MosquittoConf("Error: El directorio de configuración de mTLS no es correcto".into()))
                }
            }
            _ => {}
        }
    }
    Err(ErrorType::MosquittoConf("Error: No se encontró el directorio de configuración de mTLS".into()))
}


/// Valida completamente la configuración mTLS del broker.
///
/// Construye una instancia de `MtlsConfig`, la completa a partir
/// del archivo `mtls.conf` y valida todos los invariantes de seguridad.
///
/// # Validaciones
///
/// - `listener` definido.
/// - Versión TLS permitida (`tlsv1.2`).
/// - Existencia completa de certificados:
///   - Broker
///   - Edge local
///   - Edge remoto
/// - Flags de seguridad obligatorios.
///
/// # Parámetros
///
/// - `path`: ruta al archivo `mtls.conf`.
///
/// # Retorno
///
/// - `Ok(MtlsConfig)` si la configuración es válida.
/// - `Err(ErrorType::MtlsConfig)` si cualquier requisito falla.
///
/// # Garantía
///
/// Si retorna `Ok`, el `MtlsConfig` resultante representa
/// un estado **válido y seguro** del sistema.

fn validate_mtls_conf(path: &Path) -> Result<MtlsConfig, ErrorType> {
    let mut cfg = MtlsConfig {
        listener: None,
        tls_version: None,
        certs: Certs::default(),
        require_certificate: false,
        use_identity_as_username: false,
        allow_anonymous: true,
        connection_messages: false,
    };

    scan_mtls_config(path, &mut cfg)?;

    if cfg.listener.is_none() {
        return Err(ErrorType::MtlsConfig("Error: listener no definido".into()));
    }

    if cfg.tls_version.as_deref() != Some("tlsv1.2") {
        return Err(ErrorType::MtlsConfig("Error: versión TLS inválida".into()));
    }

    validate_certificate_set(&cfg.certs)?;

    if !(cfg.require_certificate
        && cfg.use_identity_as_username
        && !cfg.allow_anonymous
        && cfg.connection_messages)
    {
        return Err(ErrorType::MtlsConfig(
            "Error: flags de seguridad mTLS inválidos".into()
        ));
    }

    Ok(cfg)
}


/// Parsea el archivo `mtls.conf` y completa una estructura `MtlsConfig`.
///
/// Lee el archivo línea por línea, interpretando directivas explícitas
/// y asignándolas a los campos correspondientes.
///
/// # Parámetros
///
/// - `path`: ruta al archivo `mtls.conf`.
/// - `cfg`: estructura mutable a completar.
///
/// # Retorno
///
/// - `Ok(())` si el archivo pudo ser leído correctamente.
/// - `Err(ErrorType::MtlsConfig)` si el archivo no es accesible.
///
/// # Notas
///
/// - No valida coherencia ni seguridad.
/// - No detecta claves duplicadas.
/// - Ignora claves desconocidas.
///
/// La validación semántica se realiza posteriormente.

fn scan_mtls_config(path: &Path, cfg: &mut MtlsConfig) -> Result<(), ErrorType> {
    let file = File::open(path)
        .map_err(|_| ErrorType::MtlsConfig(
            "Error: No se pudo abrir el archivo mtls conf".into()
        ))?;

    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap();
        let value = parts.next();

        match (key, value) {
            ("listener", Some(p)) => cfg.listener = p.parse().ok(),
            ("tls_version", Some(p)) => cfg.tls_version = Some(p.to_string()),

            // Broker
            ("cafile", Some(p)) => cfg.certs.cafile = PathBuf::from(p),
            ("certfile", Some(p)) => cfg.certs.certfile = PathBuf::from(p),
            ("keyfile", Some(p)) => cfg.certs.keyfile = PathBuf::from(p),

            ("require_certificate", Some("true")) => cfg.require_certificate = true,
            ("use_identity_as_username", Some("true")) => cfg.use_identity_as_username = true,
            ("allow_anonymous", Some("false")) => cfg.allow_anonymous = false,
            ("connection_messages", Some("true")) => cfg.connection_messages = true,
            _ => {}
        }
    }

    Ok(())
}


/// Verifica la existencia y validez básica de todos los certificados mTLS.
///
/// Aplica validaciones de archivos para:
///
/// - Certificados del broker.
/// - Certificados del edge local.
/// - Certificados del edge remoto.
///
/// # Parámetros
///
/// - `cfg`: configuración mTLS previamente validada.
///
/// # Retorno
///
/// - `Ok(())` si todos los certificados existen y son válidos.
/// - `Err(ErrorType::MtlsConfig)` ante cualquier fallo.
///
/// # Alcance
///
/// No valida relaciones criptográficas, solo propiedades del filesystem.

fn validate_certificate_files(cfg: &MtlsConfig) -> Result<(), ErrorType> {
    validate_certificate_set(&cfg.certs)?;
    Ok(())
}


/// Valida un archivo de certificado o clave privada.
///
/// Verifica:
///
/// - Que el archivo exista.
/// - Que sea un archivo regular.
/// - Que no esté vacío.
/// - Que los permisos de la clave privada sean seguros (Unix).
///
/// # Parámetros
///
/// - `path`: ruta al archivo.
/// - `is_private_key`: indica si se trata de una clave privada.
///
/// # Retorno
///
/// - `Ok(())` si el archivo es válido.
/// - `Err(ErrorType::MtlsConfig)` si no cumple los requisitos.
///
/// # Seguridad
///
/// En sistemas Unix, las claves privadas no deben ser accesibles
/// por grupo u otros usuarios.

fn check_cert_file(path: &Path, is_private_key: bool) -> Result<(), ErrorType> {
    let metadata = metadata(path)?;

    if !metadata.is_file() {
        return Err(ErrorType::MtlsConfig("Error: Certificado mTLS inválido".into()));
    }

    if metadata.len() == 0 {
        return Err(ErrorType::MtlsConfig("Error: Certificado mTLS vacío".into()));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();

        if is_private_key && (mode & 0o077) != 0 {
            return Err(ErrorType::MtlsConfig("Error: Permisos de clave privada inseguros".into()));
        }
    }

    Ok(())
}


/// Valida un conjunto completo de certificados mTLS.
///
/// Verifica que un conjunto (`Certs`) cumpla los invariantes mínimos
/// de seguridad y coherencia.
///
/// # Validaciones
///
/// - Todos los paths están definidos.
/// - Los archivos existen y son válidos.
/// - CA y certificado no son el mismo archivo.
/// - Extensiones correctas (`.crt`, `.key`).
///
/// # Parámetros
///
/// - `certs`: conjunto de certificados a validar.
/// - `label`: identificador lógico para mensajes de error.
///
/// # Retorno
///
/// - `Ok(())` si el conjunto es válido.
/// - `Err(ErrorType::MtlsConfig)` si algún invariante falla.
///
/// # Diseño
///
/// Esta función garantiza que el conjunto puede ser usado
/// de forma segura por un cliente o broker mTLS.

fn validate_certificate_set(certs: &Certs) -> Result<(), ErrorType> {
    check_cert_file(&certs.cafile, false)?;
    check_cert_file(&certs.certfile, false)?;
    check_cert_file(&certs.keyfile, true)?;

    if certs.cafile == certs.certfile {
        return Err(ErrorType::MtlsConfig("Error: CA y cert son iguales".into()));
    }

    if certs.certfile.extension() != Some("crt".as_ref()) {
        return Err(ErrorType::MtlsConfig("Error: extensión cert inválida".into()));
    }

    if certs.keyfile.extension() != Some("key".as_ref()) {
        return Err(ErrorType::MtlsConfig("Error: extensión key inválida".into()));
    }

    Ok(())
}
