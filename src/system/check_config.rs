use which::which;
use std::process::Command;
use std::net::TcpStream;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::time::Duration;
use thiserror::Error;


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

    #[error("❌ Error de lectura/escritura (IO)")]
    Io(#[from] io::Error),

    #[error("❌ Error de lectura/escritura (IO)")]
    ClientError(#[from] rumqttc::ClientError),
}


#[derive(Debug, Default)]
pub struct MtlsConfig {
    pub listener: Option<u16>,
    pub tls_version: Option<String>,
    pub cafile: Option<PathBuf>,
    pub certfile: Option<PathBuf>,
    pub keyfile: Option<PathBuf>,
    pub require_certificate: bool,
    pub use_identity_as_username: bool,
    pub allow_anonymous: bool,
    pub connection_messages: bool,
}



pub fn check_system_config() -> Result<(), ErrorType> {

    match which("mosquitto") {
        Ok(path) => println!("✅ Éxito: El binario de mosquitto existe y es ejecutable en: {:?}", path),
        Err(_) => {
            eprintln!("❌ Error: El binario de mosquitto no existe. Instale mosquitto y vuelva a intentarlo");
            eprintln!("   Por favor instálalo con: sudo apt install mosquitto");
            return Err(ErrorType::MosquittoNotInstalled);
        },
    }

    match service_active("mosquitto") {
        Ok(_) => println!("✅ Éxito: El servicio de sistema mosquitto está activo"),
        Err(_) => {
            eprintln!("❌ Error: El servicio de sistema mosquitto no está activo");
            return Err(ErrorType::MosquittoServiceInactive);
        },
    }

    match mosquitto_listening("127.0.0.1:8883") {
        Ok(_) => println!("✅ Éxito: El servicio de sistema mosquitto está escuchando"),
        Err(_) => {
            eprintln!("❌ Error: El servicio de sistema mosquitto no está escuchando");
            return Err(ErrorType::MosquittoServiceInactive);
        },
    }

    match validate_mosquitto_conf(Path::new("/etc/mosquitto/mosquitto.conf")) {
        Ok(_) => println!("✅ Éxito: El archivo mosquitto.conf es válido"),
        Err(error) => {
            eprintln!("{}", error);
            return Err(error);
        },
    }

    let cfg = match validate_mtls_conf(Path::new("/etc/mosquitto/conf.d/mtls.conf")) {
        Ok(config) => {
            println!("✅ Éxito: Mosquitto configurado para mTLS");
            config
        },
        Err(error) => {
            eprintln!("{}", error);
            return Err(error);
        }
    };

    match validate_certificate_files(&cfg) {
        Ok(_) => println!("✅ Éxito: Los certificados mTLS son correctos"),
        Err(error) => eprintln!("{}", error),
    }

    match validate_certificate_coherence(&cfg) {
        Ok(_) => println!("✅ Éxito: Los certificados mTLS son coherentes"),
        Err(error) => {
            eprintln!("{}", error);
            return Err(ErrorType::Generic);
        },
    }

    Ok(())
}


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


fn mosquitto_listening(address: &str) -> Result<(), ErrorType> {
    let timeout = Duration::from_secs(1);
    let addr = address.parse().map_err(|_| ErrorType::Generic)?;
    TcpStream::connect_timeout(&addr, timeout)?;
    Ok(())
}


fn validate_mosquitto_conf(path: &Path) -> Result<(), ErrorType> {

    let metadata = std::fs::metadata(path).map_err(|_| ErrorType::MosquittoConf(
        "❌ Error: El archivo de configuración de mosquitto no existe o no es accesible".into()
    ))?;

    if metadata.len() == 0 {
        return Err(ErrorType::MosquittoConf("❌ Error: El archivo de configuración de mosquitto esta vacío".into()));
    }

    scan_mosquitto_file_config(&path)?;

    Ok(())
}


fn scan_mosquitto_file_config(path: &Path) -> Result<(), ErrorType> {
    let file = File::open(path)
        .map_err(|_| ErrorType::MtlsConfig("❌ Error: No se pudo abrir el archivo mosquitto conf".into()))?;

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
                    Err(ErrorType::MosquittoConf("❌ Error: El directorio de configuración de mTLS no es correcto".into()))
                }
            }
            _ => {}
        }
    }
    Err(ErrorType::MosquittoConf("❌ Error: No se encontró el directorio de configuración de mTLS".into()))
}


fn validate_mtls_conf(path: &Path) -> Result<MtlsConfig, ErrorType> {
    let mut config = MtlsConfig::default();
    scan_mtls_config(&path, &mut config)?;

    if config.listener.is_some()
        && match config.tls_version.as_deref() {
            Some("tlsv1.2") => true,
            _ => false,
        }
        && config.cafile
            .as_deref()
            .is_some_and(|p| p == Path::new("/etc/mosquitto/certs/ca.crt"))

        && config.certfile
            .as_deref()
            .is_some_and(|p| p == Path::new("/etc/mosquitto/certs/server.crt"))

        && config.keyfile
            .as_deref()
            .is_some_and(|p| p == Path::new("/etc/mosquitto/certs/server.key"))

        && config.require_certificate
        && config.use_identity_as_username
        && !config.allow_anonymous
        && config.connection_messages
    {
        Ok(config)
    } else {
        Err(ErrorType::MtlsConfig("❌ Error: No se encontraron los parámetros esperados en mtls conf".into()))
    }
}


fn scan_mtls_config(path: &Path, cfg: &mut MtlsConfig) -> Result<(), ErrorType> {
    let file = File::open(path)
        .map_err(|_| ErrorType::MtlsConfig("❌ Error: No se pudo abrir el archivo mtls conf".into()))?;
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
            ("tls_version", Some(p)) => cfg.tls_version = p.parse().ok(),
            ("cafile", Some(p)) => cfg.cafile = Some(PathBuf::from(p)),
            ("certfile", Some(p)) => cfg.certfile = Some(PathBuf::from(p)),
            ("keyfile", Some(p)) => cfg.keyfile = Some(PathBuf::from(p)),
            ("require_certificate", Some("true")) => cfg.require_certificate = true,
            ("use_identity_as_username", Some("true")) => cfg.use_identity_as_username = true,
            ("allow_anonymous", Some("false")) => cfg.allow_anonymous = false,
            ("connection_messages", Some("true")) => cfg.connection_messages = true,
            _ => {}
        }
    }

    Ok(())
}


fn validate_certificate_files(cfg: &MtlsConfig) -> Result<(), ErrorType> {
    check_cert_file(cfg.cafile.as_ref().unwrap(), false)?;
    check_cert_file(cfg.certfile.as_ref().unwrap(), false)?;
    check_cert_file(cfg.keyfile.as_ref().unwrap(), true)?;
    Ok(())
}


fn check_cert_file(path: &Path, is_private_key: bool) -> Result<(), ErrorType> {
    let metadata = fs::metadata(path)?;

    if !metadata.is_file() {
        return Err(ErrorType::MtlsConfig("❌ Error: Certificado mTLS inválido".into()));
    }

    if metadata.len() == 0 {
        return Err(ErrorType::MtlsConfig("❌ Error: Certificado mTLS vacío".into()));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();

        if is_private_key && (mode & 0o077) != 0 {
            return Err(ErrorType::MtlsConfig("❌ Error: Permisos de clave privada inseguros".into()));
        }
    }

    Ok(())
}


fn validate_certificate_coherence(cfg: &MtlsConfig) -> Result<(), ErrorType> {
    let ca = cfg.cafile.as_ref().unwrap();
    let cert = cfg.certfile.as_ref().unwrap();
    let key = cfg.keyfile.as_ref().unwrap();

    if ca == cert {
        return Err(ErrorType::MtlsConfig("❌ Error: Los certificados de CA y Server son iguales".into()));
    }

    if cert.extension() != Some("crt".as_ref()) {
        return Err(ErrorType::MtlsConfig("❌ Error: La extensión de archivo del certificado Server es inválida".into()));
    }

    if key.extension() != Some("key".as_ref()) {
        return Err(ErrorType::MtlsConfig("❌ Error: La extensión de archivo de la clave privada es inválida".into()));
    }

    Ok(())
}
