use sqlx::{FromRow};
use std::collections::HashMap;
use serde::Deserialize;
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
///
/// # Campos
/// - `networks`: Mapa de redes indexado por su ID.
/// - `topic_handshake`: tópico global para publicar handshakes.
/// - `topic_state`: tópico global para publicar el estado del sistema.
/// - `topic_heartbeat`: tópico global donde el edge publica su heartbeat.
#[derive(Debug, Clone)]
pub struct NetworkManager {
    pub networks: HashMap<String, Network>,
    pub topic_handshake: Topic,
    pub topic_state: Topic,
    pub topic_heartbeat: Topic,
}


impl NetworkManager {
    /// Crea una nueva instancia vacía del gestor.
    /// Los tópicos de handshake y state son globales, independientes de la red.
    /// Por ende tienen un path definido. Siempre es `iot/id_edge/handshake` o `iot/id_edge/state`
    /// Ademas el QoS es 1. Esto significa `AtLeastOnce`.
    pub fn new(system: &System, networks: HashMap<String, Network>) -> Self {
        let id_system = system.id_edge.clone();
        let t_handshake = format!("iot/{id_system}/handshake");
        let t_state = format!("iot/{id_system}/state");
        let t_heartbeat = format!("iot/{id_system}/heartbeat");
        Self {
            networks,
            topic_handshake: Topic::new(t_handshake, 1),
            topic_state: Topic::new(t_state, 1),
            topic_heartbeat: Topic::new(t_heartbeat, 0),
        }
    }

    pub fn new_empty(system: &System) -> Self {
        let id_system = system.id_edge.clone();
        let t_handshake = format!("iot/{id_system}/handshake");
        let t_state = format!("iot/{id_system}/state");
        let t_heartbeat = format!("iot/{id_system}/heartbeat");
        Self {
            networks: HashMap::new(),
            topic_handshake: Topic::new(t_handshake, 1),
            topic_state: Topic::new(t_state, 1),
            topic_heartbeat: Topic::new(t_heartbeat, 0),
        }
    }

    /// Agrega o actualiza una red en la memoria.
    pub fn add_network(&mut self, network: Network) {
        self.networks.insert(network.id_network.clone(), network);
    }
    
    pub fn change_active(&mut self, active: bool, id: &str) {
        if self.networks.get(id).is_some() {
            self.networks.get_mut(id).unwrap().active = active;
        }
    }
    
    /// Elimina una red de la memoria.
    pub fn remove_network(&mut self, id: &str) {
        self.networks.remove(id);
    }

    /// Transforma un tópico de entrada en una ruta de respuesta estandarizada.
    ///
    /// Además, actualiza el valor de `qos_topic` por referencia si encuentra
    /// una coincidencia en la configuración. El tópico retornado es donde el Edge debe
    /// publicar el mensaje correspondiente al servidor o hub.
    ///
    /// # Formato esperado
    /// Se espera que `topic_in` tenga el formato: `prefijo/red/nodo/tipo`
    ///
    /// # Retorno
    /// - `Some(String)`: `iot/{red}/{system_id}/{tipo}` si la red existe.
    /// - `None`: Si el formato es inválido o la red no existe.
    pub fn topic_to_send_msg(&self, topic_in: &str, system: &System) -> Option<Topic> {

        let parts: Vec<&str> = topic_in.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let net = parts[1];
        let type_msg = parts[3];
        let id = &system.id_edge;
        let qos_topic : u8;

        if let Some(n) = self.networks.get(net) {
            let pairs = [
                (&n.topic_data.topic, n.topic_data.qos),
                (&n.topic_alert_air.topic, n.topic_alert_air.qos),
                (&n.topic_alert_temp.topic, n.topic_alert_temp.qos),
                (&n.topic_monitor.topic, n.topic_monitor.qos),
                (&n.topic_network.topic, n.topic_network.qos),
                (&n.topic_new_setting_to_edge.topic, n.topic_new_setting_to_edge.qos),
                (&n.topic_new_setting_to_hub.topic, n.topic_new_setting_to_hub.qos),
                (&n.topic_hub_settings_ok.topic, n.topic_hub_settings_ok.qos),
                (&n.topic_new_firmware.topic, n.topic_new_firmware.qos),
                (&n.topic_hub_firmware_ok.topic, n.topic_hub_firmware_ok.qos),
                (&n.topic_balance_mode_handshake.topic, n.topic_balance_mode_handshake.qos),
            ];

            if let Some((_, qos)) = pairs.iter().find(|(pattern, _)| topic_matches(pattern, topic_in)) {
                qos_topic = *qos;
            } else {
                // Si no coincide con ninguno de los patrones configurados, no hay un QoS para darle
                return None
            }

            Some(Topic::new(format!("iot/{net}/{id}/{type_msg}"), qos_topic))
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
            if topic_matches(&n.topic_data.topic, topic_in) {
                return Table::Measurement;
            }
            if topic_matches(&n.topic_monitor.topic, topic_in) {
                return Table::Monitor;
            }
            if topic_matches(&n.topic_alert_air.topic, topic_in)  {
                return Table::AlertAir;
            }
            if topic_matches(&n.topic_alert_temp.topic, topic_in) {
                return Table::AlertTemp;
            }
            Table::Error
        }).unwrap_or(Table::Error)
    }
}

fn topic_matches(pattern: &str, topic: &str) -> bool {
    let mut p_iter = pattern.split('/');
    let mut t_iter = topic.split('/');

    loop {
        match (p_iter.next(), t_iter.next()) {
            // Caso 1: Ambos tienen un segmento para comparar
            (Some(p), Some(t)) => {
                if p != "+" && p != t {
                    return false; // No coinciden y no es comodín
                }
            },
            // Caso 2: Ambos terminaron al mismo tiempo (son iguales)
            (None, None) => return true,

            // Caso 3: Uno terminó y el otro no (Longitudes diferentes)
            _ => return false,
        }
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
/// - `id_network`: id único de la red.
/// - `name_network`: nombre de la red (mas descriptivo que el id).
/// - `topic_data`: tópico donde se suscribe el Edge para recibir datos.
/// - `topic_alert_air`: tópico donde se suscribe el Edge para recibir alertas de aire.
/// - `topic_alert_temp`: tópico donde se suscribe el Edge para recibir alertas de temperatura.
/// - `topic_monitor`: tópico donde se suscribe el Edge para recibir mensajes de monitoreo.
/// - `topic_network`: tópico donde se suscribe el Edge para recibir mensajes del servidor para modificar redes.
/// - `topic_new_setting_to_edge`: tópico donde se suscribe el Edge para recibir mensajes del servidor para configurarse.
/// - `topic_new_setting_to_hub`: tópico donde se suscribe el Edge para recibir mensajes del servidor para configurar hubs.
/// - `topic_hub_settings_ok`: tópico donde se suscribe el Edge para recibir la confirmación de config aplicada de los hubs.
/// - `topic_new_firmware`: tópico donde se suscribe el Edge para recibir mensajes del servidor para actualizar el firmware de los hubs.
/// - `topic_hub_firmware_ok`: tópico donde se suscribe el Edge para recibir el handshake del hub de nuevo firmware listo.
/// - `topic_balance_mode_handshake`: tópico donde se suscribe el Edge para recibir mensaje de handshake de los nodos (en balance mode).
/// - `topic_hello_world`: tópico donde publica el Edge que está configurado y listo cuando inicia (al servidor).
/// - `active`: variable que indica si la red actualmente está activa o inactiva.
#[derive(Debug, Clone)]
pub struct Network {
    pub id_network: String,
    pub name_network: String,
    pub topic_data: Topic,
    pub topic_alert_air: Topic,
    pub topic_alert_temp: Topic,
    pub topic_monitor: Topic,
    pub topic_network: Topic,
    pub topic_new_setting_to_edge: Topic,
    pub topic_new_setting_to_hub: Topic,
    pub topic_hub_settings_ok: Topic,
    pub topic_new_firmware: Topic,
    pub topic_hub_firmware_ok: Topic,
    pub topic_balance_mode_handshake: Topic,
    pub topic_hello_world: Topic,
    pub active: bool,
}


impl Network {
    pub fn new(id_network: String, name_network: String, active: bool, system: &System) -> Self {
        let id_system = system.id_edge.clone();
        let t_data = format!("iot/{id_network}/+/data");
        let t_alert_air = format!("iot/{id_network}/+/alert_air");
        let t_alert_temp = format!("iot/{id_network}/+/alert_temp");
        let t_monitor = format!("iot/{id_network}/+/monitor");
        let t_network = format!("iot/{id_network}/{id_system}/network");
        let t_new_setting_to_edge = format!("iot/{id_network}/{id_system}/new_setting_to_edge");
        let t_new_setting_to_hub = format!("iot/{id_network}/{id_system}/new_setting_to_hub");
        let t_hub_setting_ok = format!("iot/{id_network}/+/hub_setting_ok");
        let t_new_firmware = format!("iot/{id_network}/{id_system}/new_firmware");
        let t_hub_firmware_ok = format!("iot/{id_network}/+/hub_firmware_ok");
        let t_balance_mode_handshake = format!("iot/{id_network}/+/balance_mode_handshake");
        let t_hello_world = format!("iot/{id_network}/{id_system}/hello_world");

        Self {
            id_network,
            name_network,
            topic_data: Topic::new(t_data, 0),
            topic_alert_air: Topic::new(t_alert_air, 1),
            topic_alert_temp: Topic::new(t_alert_temp, 1),
            topic_monitor: Topic::new(t_monitor, 0),
            topic_network: Topic::new(t_network, 0),
            topic_new_setting_to_edge: Topic::new(t_new_setting_to_edge, 0),
            topic_new_setting_to_hub: Topic::new(t_new_setting_to_hub, 0),
            topic_hub_settings_ok: Topic::new(t_hub_setting_ok, 0),
            topic_new_firmware: Topic::new(t_new_firmware, 0),
            topic_hub_firmware_ok: Topic::new(t_hub_firmware_ok, 0),
            topic_balance_mode_handshake: Topic::new(t_balance_mode_handshake, 0),
            topic_hello_world: Topic::new(t_hello_world, 0),
            active
        }
    }
}



/// Modelo de datos plano para mapeo directo con SQLx (SQLite).
#[derive(Debug, FromRow, Deserialize)]
pub struct NetworkRow {
    pub id_network: String,
    pub name_network: String,
    pub active: bool,
}


/// Convierte una fila plana de base de datos (`NetworkRow`) a la estructura jerárquica (`Network`).
impl NetworkRow {
    pub fn cast_to_network(self, system: &System) -> Network {
        Network::new(self.id_network, self.name_network, self.active, &system)
    }
}


pub enum NetworkAction {
    Delete,
    Update,
    Insert,
    Ignore,
}