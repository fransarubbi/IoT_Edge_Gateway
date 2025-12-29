use std::fs;
use std::fs::File;
use std::io::{Write};
use std::process::Command;
use std::ffi::CString;
use libc::{chown, uid_t, gid_t};
use std::os::unix::fs::PermissionsExt;
use crate::system::fsm::Flag;
use super::check_config::ErrorType;



pub fn configurate_system(event: &Flag) -> Result<(), ErrorType> {

    match event {
        Flag::MosquittoServiceInactive => {
            match activate_service() {
                Ok(_) => println!("✅ Éxito: Servicio mosquitto activado correctamente"),
                Err(_) => eprintln!("❌ Error: Systemctl falló al activar mosquitto"),
            }
            
            match create_mosquitto_conf() {
                Ok(_) => println!("✅ Éxito: Archivo de configuración de mosquitto creado correctamente"),
                Err(_) => eprintln!("❌ Error: No se pudo crear el archivo de configuración de mosquitto"),
            }

            match create_mtls_config_file() {
                Ok(_) => println!("✅ Éxito: Archivo de configuración de mTLS creado correctamente"),
                Err(_) => eprintln!("❌ Error: No se pudo crear el archivo de configuración de mTLS"),
            }

            match restart_service() {
                Ok(_) => println!("✅ Éxito: Servicio mosquitto reiniciado correctamente"),
                Err(_) => eprintln!("❌ Error: Systemctl falló al reiniciar mosquitto"),
            }
        },
        Flag::MosquittoConf => {
            match create_mosquitto_conf() {
                Ok(_) => println!("✅ Éxito: Archivo de configuración de mosquitto creado correctamente"),
                Err(_) => eprintln!("❌ Error: No se pudo crear el archivo de configuración de mosquitto"),
            }

            match create_mtls_config_file() {
                Ok(_) => println!("✅ Éxito: Archivo de configuración de mTLS creado correctamente"),
                Err(_) => eprintln!("❌ Error: No se pudo crear el archivo de configuración de mTLS"),
            }

            match restart_service() {
                Ok(_) => println!("✅ Éxito: Servicio mosquitto reiniciado correctamente"),
                Err(_) => eprintln!("❌ Error: Systemctl falló al reiniciar mosquitto"),
            }
        },
        Flag::MtlsConf => {
            match create_mtls_config_file() {
                Ok(_) => println!("✅ Éxito: Archivo de configuración de mTLS creado correctamente"),
                Err(_) => eprintln!("❌ Error: No se pudo crear el archivo de configuración de mTLS"),
            }

            match restart_service() {
                Ok(_) => println!("✅ Éxito: Servicio mosquitto reiniciado correctamente"),
                Err(_) => eprintln!("❌ Error: Systemctl falló al reiniciar mosquitto"),
            }
        },
        _ => {},
    }
    
    Ok(())
}


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


fn create_mosquitto_conf() -> Result<(), ErrorType> {
    let path = "/etc/mosquitto";
    let path_file = "/etc/mosquitto/mosquitto.conf";

    if let Err(e) = fs::create_dir_all(path) {
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


fn create_mtls_config_file() -> Result<(), ErrorType> {
    let path = "/etc/mosquitto/conf.d";
    let path_file = "/etc/mosquitto/conf.d/mtls.conf";

    if let Err(e) = fs::create_dir_all(path) {
        return Err(ErrorType::Generic);
    }

    // File::create crea o vacia el archivo si ya existe
    let mut file = File::create(path_file)?;

    // Usamos r#""# porque permite texto multilinea
    let text = r#"# Configuracion para IoT Edge Gateway
listener 8883
tls_version tlsv1.2
cafile /etc/mosquitto/certs/ca.crt
certfile /etc/mosquitto/certs/server.crt
keyfile /etc/mosquitto/certs/server.key
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


fn chown_root(path: &str) -> Result<(), ErrorType> {
    let c_path = CString::new(path).unwrap();
    let res = unsafe { chown(c_path.as_ptr(), 0 as uid_t, 0 as gid_t) };

    if res == 0 {
        Ok(())
    } else {
        Err(ErrorType::Generic)
    }
}
