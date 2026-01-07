use sqlx::{FromRow};
use std::collections::HashMap;
use crate::database::domain::Table;
use crate::system::domain::System;


/// Gestor en memoria de las configuraciones de redes y tópicos del sistema.
///
/// Este struct actúa como una caché de lectura rápida para evitar consultar
/// la base de datos cada vez que llega un mensaje MQTT.
///
/// # Responsabilidades
/// - Almacenar la configuración de cada red (`Network`).
/// - Validar si un tópico entrante pertenece a una red conocida.
/// - Mapear tópicos de entrada a tipos de tablas (`Table`).
/// - Calcular el QoS correcto para las respuestas.
#[derive(Debug, Clone)]
pub struct NetworkManager {
    /// Mapa de redes indexado por su ID (ejemplo: "sala7").
    pub networks: HashMap<String, Network>,
    /// Tópico global para el handshake inicial.
    topic_handshake: Topic,
    /// Tópico global para reportar el estado del sistema.
    topic_state: Topic,
}

impl NetworkManager {
    /// Crea una nueva instancia vacía del gestor.
    pub fn new(topic_h: String, qos_h: u8, topic_s: String, qos_s: u8) -> Self {
        Self {
            networks: HashMap::new(),
            topic_handshake: Topic::new(topic_h, qos_h),
            topic_state: Topic::new(topic_s, qos_s),
        }
    }

    /// Agrega o actualiza una red en la memoria.
    pub fn add_network(&mut self, network: Network) {
        self.networks.insert(network.id_network.clone(), network);
    }

    /// Transforma un tópico de entrada en una ruta de respuesta estandarizada.
    ///
    /// Además, actualiza el valor de `qos_topic` por referencia si encuentra
    /// una coincidencia en la configuración.
    ///
    /// # Formato esperado
    /// Se espera que `topic_in` tenga el formato: `prefijo/red/nodo/tipo`
    ///
    /// # Retorno
    /// - `Some(String)`: `iot/{red}/{system_id}/{tipo}` si la red existe.
    /// - `None`: Si el formato es inválido o la red no existe.
    pub fn topic_to_send_msg(&self, topic_in: &str, system: &System, qos_topic: &mut u8) -> Option<String> {
        let mut iter = topic_in.split('/');

        // 0="iot", 1="sala7"(net), 2="nodo3", 3="sensors"(type)
        let net = iter.nth(1)?;      // index 1
        let type_msg = iter.nth(1)?; // index 1 + 2 = 3 (iter avanza)
        let id = &system.id_edge;

        if let Some(n) = self.networks.get(net) {
            // Array de tuplas para iterar y buscar coincidencia de QoS
            let pairs = [
                (&n.topic_data.topic, n.topic_data.qos),
                (&n.topic_alert_air.topic, n.topic_alert_air.qos),
                (&n.topic_alert_temp.topic, n.topic_alert_temp.qos),
                (&n.topic_monitor.topic, n.topic_monitor.qos),
                (&n.topic_settings_from_hub.topic, n.topic_settings_from_hub.qos),
                (&n.topic_settings_from_ser.topic, n.topic_settings_from_ser.qos),
                (&n.topic_active.topic, n.topic_active.qos),
            ];

            // Si el tópico de entrada coincide con alguno configurado, copiamos su QoS
            if let Some((_, qos)) = pairs.iter().find(|(t, _)| topic_in == *t) {
                *qos_topic = *qos;
            }

            Some(format!("iot/{net}/{id}/{type_msg}"))
        } else {
            None
        }
    }

    /// Extrae el ID de la red de un tópico si esta existe en el gestor.
    ///
    /// # Ejemplo
    /// `iot/sala7/nodo1` -> Retorna `Some("sala7")` si "sala7" está en `networks`.
    pub fn extract_net_id(&self, topic_in: &str) -> Option<String> {
        let mut iter = topic_in.split('/');
        let net = iter.nth(1)?; // Obtiene el segundo elemento

        if self.networks.contains_key(net) {
            Some(net.to_string())
        } else {
            None
        }
    }

    /// Determina a qué tabla de base de datos corresponde un tópico.
    ///
    /// Compara el tópico de entrada con los tópicos configurados para esa red
    /// (datos, monitor, alertas, etc.).
    ///
    /// # Retorno
    /// - `Table::[Tipo]`: Si hay coincidencia.
    /// - `Table::Error`: Si la red no existe o el tópico no coincide con ninguno conocido.
    pub fn extract_topic(&self, topic_in: &str, id: &str) -> Table {
        self.networks.get(id).map(|n| {
            if topic_in == n.topic_data.topic {
                return Table::Measurement;
            }
            if topic_in == n.topic_monitor.topic {
                return Table::Monitor;
            }
            if topic_in == n.topic_alert_air.topic {
                return Table::AlertAir;
            }
            if topic_in == n.topic_alert_temp.topic {
                return Table::AlertTemp;
            }
            Table::Error
        }).unwrap_or(Table::Error)
    }
}


/// Representación simple de un Tópico MQTT y su calidad de servicio (QoS).
#[derive(Debug, Clone)]
pub struct Topic {
    pub topic: String,
    pub qos: u8,
}


impl Topic {
    pub fn new(topic: String, qos: u8) -> Self {
        Self { topic, qos }
    }
}


/// Configuración completa de una red en memoria.
///
/// Contiene todos los tópicos asociados a una red específica.
#[derive(Debug, Clone)]
pub struct Network {
    pub id_network: String,
    pub name_network: String,
    pub topic_data: Topic,
    pub topic_alert_air: Topic,
    pub topic_alert_temp: Topic,
    pub topic_monitor: Topic,
    pub topic_settings_from_hub: Topic,
    pub topic_settings_from_ser: Topic,
    pub topic_active: Topic,
    pub active: bool,
}


/// Convierte una fila plana de base de datos (`NetworkRow`) a la estructura jerárquica (`Network`).
impl From<NetworkRow> for Network {
    fn from(row: NetworkRow) -> Self {
        Self {
            id_network: row.id_network,
            name_network: row.name_network,
            topic_data: Topic::new(row.topic_data, row.topic_data_qos),
            topic_alert_air: Topic::new(row.topic_alert_air, row.topic_alert_air_qos),
            topic_alert_temp: Topic::new(row.topic_alert_temp, row.topic_alert_temp_qos),
            topic_monitor: Topic::new(row.topic_monitor, row.topic_monitor_qos),
            topic_settings_from_hub: Topic::new(row.topic_settings_hub, row.topic_settings_qos_hub),
            topic_settings_from_ser: Topic::new(row.topic_settings_ser, row.topic_settings_qos_ser),
            topic_active: Topic::new(row.topic_active, row.topic_active_qos),
            active: row.active,
        }
    }
}


/// Modelo de datos plano para mapeo directo con SQLx (SQLite).
#[derive(Debug, FromRow)]
pub struct NetworkRow {
    pub id_network: String,
    pub name_network: String,
    pub topic_data: String,
    pub topic_data_qos: u8,
    pub topic_alert_air: String,
    pub topic_alert_air_qos: u8,
    pub topic_alert_temp: String,
    pub topic_alert_temp_qos: u8,
    pub topic_monitor: String,
    pub topic_monitor_qos: u8,
    pub topic_settings_hub: String,
    pub topic_settings_qos_hub: u8,
    pub topic_settings_ser: String,
    pub topic_settings_qos_ser: u8,
    pub topic_active: String,
    pub topic_active_qos: u8,
    pub active: bool,
}