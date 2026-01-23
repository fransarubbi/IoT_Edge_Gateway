//! # Módulo de Recolección de Métricas (Monitor)
//!
//! Este módulo se encarga de interactuar con el hardware y el sistema operativo
//! para extraer métricas en tiempo real. Está optimizado para sistemas Linux
//! (específicamente Raspberry Pi) mediante la lectura directa de archivos del kernel
//!
//! ## Características
//! - Persistencia de conexiones con `sysinfo` para optimizar rendimiento.
//! - Lectura manual de temperatura térmica (Thermal Zone 0).
//! - Parsing manual de calidad de señal WiFi (RSSI/dBm).


use sysinfo::{System, Disks, Networks};
use std::fs;
use crate::message::domain::SystemMetrics;


/// Recolector de estado del sistema.
///
/// Mantiene las instancias de las estructuras de `sysinfo` para evitar
/// la realocación de memoria en cada ciclo de lectura. Es necesario instanciar
/// esta estructura una sola vez y mantenerla viva durante la ejecución del programa.
pub struct MetricsCollector {
    system: System,
    disks: Disks,
    networks: Networks,
}


/// Representación interna de la señal inalámbrica.
pub struct WifiSignal {
    pub rssi: i32,
    pub dbm: i32,
}


impl MetricsCollector {

    /// Crea una nueva instancia del recolector.
    ///
    /// Inicializa y escanea todos los componentes de hardware disponibles.
    /// Esta operación puede tardar unos milisegundos.
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_all();

        let disks = Disks::new_with_refreshed_list();
        let networks = Networks::new_with_refreshed_list();

        Self {
            system,
            disks,
            networks,
        }
    }

    /// Actualiza y retorna las métricas actuales del sistema.
    ///
    /// Este método refresca la información de hardware de forma incremental
    /// (solo actualiza valores, no re-escanea listas de dispositivos) y calcula
    /// promedios cuando es necesario.
    ///
    /// # Retorno
    /// Retorna un `SystemMetrics` (DTO) listo para ser enviado o almacenado.
    pub fn collect(&mut self) -> SystemMetrics {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.disks.refresh(false);
        self.networks.refresh(false);

        // Cálculo de CPU (promedio de todos los núcleos)
        let cpu_usage_percent = {
            let cpus = self.system.cpus();
            if cpus.is_empty() {
                0.0
            } else {
                cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32
            }
        };

        // Temperatura (lectura directa de archivo para mayor precisión en RPi)
        let cpu_temp_celsius = read_cpu_temperature().unwrap_or(0.0);

        let ram_total_mb = self.system.total_memory() / (1024 * 1024);
        let ram_used_mb = self.system.used_memory() / (1024 * 1024);

        // Almacenamiento (buscando partición raíz /)
        let mut sd_total_gb = 0;
        let mut sd_used_gb = 0;
        let mut sd_usage_percent = 0.0;

        for disk in self.disks.iter() {
            if disk.mount_point() == std::path::Path::new("/") {
                let total = disk.total_space();
                let used = total - disk.available_space();

                sd_total_gb = total / 1024 / 1024 / 1024;
                sd_used_gb = used / 1024 / 1024 / 1024;
                sd_usage_percent = (used as f32 / total as f32) * 100.0;
                break;
            }
        }

        // Red (acumulado de todas las interfaces)
        let mut network_rx_bytes = 0;
        let mut network_tx_bytes = 0;

        for (_, data) in self.networks.iter() {
            network_rx_bytes += data.received();
            network_tx_bytes += data.transmitted();
        }

        // WiFi (específico para interfaz wlan0)
        let wifi = read_wifi_signal("wlan0");
        let wifi_rssi = wifi.as_ref().map(|w| w.rssi);
        let wifi_signal_dbm  = wifi.as_ref().map(|w| w.dbm);


        let uptime_seconds = System::uptime();

        SystemMetrics {
            uptime_seconds,
            cpu_usage_percent,
            cpu_temp_celsius,
            ram_total_mb,
            ram_used_mb,
            sd_total_gb,
            sd_used_gb,
            sd_usage_percent,
            network_rx_bytes,
            network_tx_bytes,
            wifi_rssi,
            wifi_signal_dbm
        }
    }
}


/// Lee la temperatura de la CPU directamente desde el sistema de archivos virtual.
///
/// Intenta leer `/sys/class/thermal/thermal_zone0/temp`.
///
/// # Retorno
/// * `Some(f32)`: Temperatura en grados Celsius.
/// * `None`: Si el archivo no existe o no se puede parsear.
fn read_cpu_temperature() -> Option<f32> {
    let raw = fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").ok()?;
    let millideg: f32 = raw.trim().parse().ok()?;
    Some(millideg / 1000.0)
}


/// Lee la calidad de la señal WiFi desde `/proc/net/wireless`.
///
/// # Argumentos
/// * `interface`: Nombre de la interfaz (ej. "wlan0").
///
/// # Parsing
/// Parsea el formato estándar de Linux Wireless Extensions

// Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE
//  face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22
// wlan0: 0000   70.   -42.  -256     0      0       0      0      0        0
fn read_wifi_signal(interface: &str) -> Option<WifiSignal> {
    let content = fs::read_to_string("/proc/net/wireless").ok()?;

    for line in content.lines().skip(2) {  // Saltamos las 2 primeras líneas de cabecera
        if line.trim_start().starts_with(interface) {
            let parts: Vec<&str> = line.split_whitespace().collect();

            // Índice 2: Link Quality
            // Índice 3: Signal Level (dBm)
            let rssi = parts.get(2)?.trim_end_matches('.').parse().ok()?;
            let dbm  = parts.get(3)?.trim_end_matches('.').parse().ok()?;

            return Some(WifiSignal { rssi, dbm });
        }
    }
    None
}