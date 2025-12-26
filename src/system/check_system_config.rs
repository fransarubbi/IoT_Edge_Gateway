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
    Generic,

    #[error("{0}")]
    Config(String),

    #[error("❌ Error de lectura/escritura (IO)")]
    Io(#[from] io::Error),
}


#[derive(Debug, Default)]
pub struct MtlsConfig {
    pub listener: Option<u16>,
    pub cafile: Option<PathBuf>,
    pub certfile: Option<PathBuf>,
    pub keyfile: Option<PathBuf>,
    pub require_certificate: bool,
    pub use_identity_as_username: bool,
}



pub fn check_system_config() {

    match which("mosquitto") {
        Ok(path) => println!("✅ Éxito: El binario de mosquitto existe y es ejecutable en: {:?}", path),
        Err(_) => println!("❌ Fallo: El binario de mosquitto no existe"),
    }

    match service_active("mosquitto") {
        Ok(_) => println!("✅ Éxito: El servicio de sistema mosquitto está activo"),
        Err(_) => println!("❌ Fallo: El servicio de sistema mosquitto no está activo"),
    }

    match mosquitto_listening("127.0.0.1:8883") {
        Ok(_) => println!("✅ Éxito: El servicio de sistema mosquitto está escuchando"),
        Err(_) => println!("❌ Fallo: El servicio de sistema mosquitto no está escuchando"),
    }

    match validate_mosquitto_file_config("/etc/mosquitto/mosquitto.conf") {
        Ok(_) => println!("✅ Éxito: El archivo conf es válido (existe y con contenido)"),
        Err(error) => println!("{}", error),
    }

    let cfg = match check_mtls_config(Path::new("/etc/mosquitto/mosquitto.conf")) {
        Ok(config) => {
            println!("✅ Éxito: Mosquitto configurado para mTLS");
            config
        },
        Err(e) => {
            println!("❌ FALLO: Mosquitto no está configurado para exigir certificados mTLS");
            return;
        }
    };

    match check_certificate_files(&cfg) {
        Ok(_) => println!("✅ ÉXITO: Los certificados mTLS correctos"),
        Err(error) => println!("{}", error),
    }

    match check_certificate_coherence(&cfg) {
        Ok(_) => println!("✅ ÉXITO: Los certificados mTLS son coherentes"),
        Err(error) => println!("{}", error),
    }
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
    match TcpStream::connect_timeout(&address.parse().unwrap(), timeout) {
        Ok(_) => Ok(()),
        Err(_) => Err(ErrorType::Generic),
    }
}


fn validate_mosquitto_file_config(ruta: &str) -> Result<(), ErrorType> {
    let path = Path::new(ruta);

    let metadata = std::fs::metadata(path).map_err(|_| ErrorType::Config(
        "❌ FALLO: El archivo de configuración de mosquitto no existe o no es accesible".into()
    ))?;

    if metadata.len() == 0 {
        return Err(ErrorType::Config("❌ FALLO: El archivo de configuración de mosquitto esta vacío".into()));
    }

    Ok(())
}


pub fn check_mtls_config(main_conf: &Path) -> Result<MtlsConfig, ErrorType> {
    let mut config = MtlsConfig::default();
    scan_mosquitto_config(main_conf, &mut config)?;

    if config.listener.is_some()
        && config.cafile.is_some()
        && config.certfile.is_some()
        && config.keyfile.is_some()
        && config.require_certificate
    {
        Ok(config)
    } else {
        Err(ErrorType::Generic)
    }
}


fn scan_mosquitto_config(path: &Path, cfg: &mut MtlsConfig) -> Result<(), ErrorType> {
    let file = File::open(path)?;
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
            ("cafile", Some(p)) => cfg.cafile = Some(PathBuf::from(p)),
            ("certfile", Some(p)) => cfg.certfile = Some(PathBuf::from(p)),
            ("keyfile", Some(p)) => cfg.keyfile = Some(PathBuf::from(p)),
            ("require_certificate", Some("true")) => cfg.require_certificate = true,
            ("use_identity_as_username", Some("true")) => {
                cfg.use_identity_as_username = true
            }

            ("include_file", Some(p)) => {
                scan_mosquitto_config(Path::new(p), cfg)?;
            }

            ("include_dir", Some(dir)) => {
                scan_include_dir(Path::new(dir), cfg)?;
            }

            _ => {}
        }
    }

    Ok(())
}


fn scan_include_dir(dir: &Path, cfg: &mut MtlsConfig) -> Result<(), ErrorType> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) == Some("conf") {
            scan_mosquitto_config(&path, cfg)?;
        }
    }

    Ok(())
}


pub fn check_certificate_files(cfg: &MtlsConfig) -> Result<(), ErrorType> {
    check_cert_file(cfg.cafile.as_ref().unwrap(), false)?;
    check_cert_file(cfg.certfile.as_ref().unwrap(), false)?;
    check_cert_file(cfg.keyfile.as_ref().unwrap(), true)?;
    Ok(())
}


fn check_cert_file(path: &Path, is_private_key: bool) -> Result<(), ErrorType> {
    let metadata = fs::metadata(path)?;

    if !metadata.is_file() {
        return Err(ErrorType::Config("❌ FALLO: Certificado mTLS inválido".into()));
    }

    if metadata.len() == 0 {
        return Err(ErrorType::Config("❌ FALLO: Certificado mTLS vacío".into()));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();

        if is_private_key && (mode & 0o077) != 0 {
            return Err(ErrorType::Config("❌ FALLO: Permisos de clave privada inseguros".into()));
        }
    }

    Ok(())
}


pub fn check_certificate_coherence(cfg: &MtlsConfig) -> Result<(), ErrorType> {
    let ca = cfg.cafile.as_ref().unwrap();
    let cert = cfg.certfile.as_ref().unwrap();
    let key = cfg.keyfile.as_ref().unwrap();

    if ca == cert {
        return Err(ErrorType::Config("❌ FALLO: Los certificados de CA y Server son iguales".into()));
    }

    if cert.extension() != Some("crt".as_ref()) {
        return Err(ErrorType::Config("❌ FALLO: La extensión de archivo del certificado Server es inválida".into()));
    }

    if key.extension() != Some("key".as_ref()) {
        return Err(ErrorType::Config("❌ FALLO: La extensión de archivo de la clave privada es inválida".into()));
    }

    Ok(())
}
