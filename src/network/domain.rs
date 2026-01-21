use sqlx::{FromRow};
use std::collections::{HashMap, HashSet};
use serde::Deserialize;
use tracing::{info, warn};
use crate::database::domain::Table;
use crate::message::domain_for_table::HubRow;
use crate::system::domain::System;
use rand::seq::IteratorRandom;


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
    pub hubs: HashMap<String, HashSet<Hub>>,
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
            hubs: HashMap::new(),
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
            hubs: HashMap::new(),
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

    /// Agrega o actualiza un hub en memoria.
    pub fn add_hub(&mut self, id: String, hub: Hub) {
        // 1. Busca la entrada por id.
        // 2. Si no existe, crea un HashSet vacío (or_default).
        // 3. Intenta insertar el hub.
        let is_new = self.hubs
            .entry(id)
            .or_default()
            .insert(hub);

        if is_new {
            info!("Hub agregado.");
        } else {
            warn!("El Hub ya existía en esta red, fue ignorado.");
        }
    }

    /// Eliminar todos los Hubs asociados a una red.
    pub fn remove_hub_network(&mut self, id: &str) {
        self.hubs.remove(id);
    }

    /// Eliminar un Hub específico de una red.
    pub fn remove_hub(&mut self, id_network: &str, id_hub: &str) {
        if let Some(hubs_set) = self.hubs.get_mut(id_network) {
            let len_before = hubs_set.len();
            hubs_set.retain(|hub| hub.id != id_hub);

            if hubs_set.len() < len_before {
                info!("Hub '{}' eliminado de la red '{}'.", id_hub, id_network);
            } else {
                warn!("El Hub '{}' no existía en la red '{}'.", id_hub, id_network);
            }
        } else {
            warn!("Intento de borrar hub en red inexistente: '{}'.", id_network);
        }
    }

    /// Preguntar si existe un determinado Hub por ID.
    pub fn search_hub(&self, id_net: &str, id_hub: &str) -> bool {
        if let Some(hubs_set) = self.hubs.get(id_net) {
            return hubs_set.iter().any(|hub| hub.id == id_hub);
        }
        true  // si la red no existe, retorna true para no guardar datos en network_task
    }

    /// Obtener un id de un Hub aleatorio perteneciente a una determinada red.
    pub fn get_random_hub_id_by_network(&self, id_net: &str) -> Option<String> {
        
        let hubs_set = self.hubs.get(id_net)?;
        let random_hub = hubs_set.iter().choose(&mut rand::thread_rng())?;
        Some(random_hub.id.clone())
    }

    /// Obtener la cantidad de Hubs asociados a una red en particular.
    pub fn get_total_hubs_by_network(&self, id_net: &str) -> Option<usize> {
        let hubs_set = self.hubs.get(id_net)?;
        Some(hubs_set.len())
    }

    /// Transforma un tópico local (proveniente del Hub) en un tópico de destino para el Servidor.
    ///
    /// Esta función actúa como un **Gateway de Salida** (Uplink). Toma mensajes generados
    /// en la red local y los reempaqueta estandarizándolos con la identidad del Edge
    /// antes de subirlos a la nube.
    ///
    /// # Lógica de Transformación
    ///
    /// Extrae dinámicamente el tipo de mensaje (el sufijo del tópico) y lo preserva,
    /// pero inyecta el `id_edge` en la ruta.
    ///
    /// - **Entrada:** `iot/{red}/hub/{id_nodo}/{tipo}`
    /// - **Salida:** `iot/{red}/edge/{id_edge}/{tipo}`
    ///
    /// # Proceso de Validación
    ///
    /// 1. Verifica que el tópico tenga al menos 4 segmentos.
    /// 2. Busca si la red (segmento 1) existe en memoria.
    /// 3. Compara el tópico de entrada contra la lista de patrones configurados
    ///    (Datos, Alertas, Monitor, Handshakes) usando coincidencia con wildcards (`topic_matches`).
    /// 4. Si hay coincidencia, asigna el QoS configurado para ese tipo de mensaje.
    ///
    /// # Retorno
    ///
    /// - `Some(Topic)`: Con el nuevo string de tópico y el QoS correcto.
    /// - `None`: Si la red no existe, el tópico es malformado o no coincide con ninguna configuración.
    pub fn get_topic_to_send_msg_from_hub(&self, topic_in: &str, id: &str) -> Option<Topic> {

        let parts: Vec<&str> = topic_in.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let net = parts[1];
        let type_msg = parts[4];
        let qos_topic : u8;

        if let Some(n) = self.networks.get(net) {
            let pairs = [
                (&n.topic_data.topic, n.topic_data.qos),
                (&n.topic_alert_air.topic, n.topic_alert_air.qos),
                (&n.topic_alert_temp.topic, n.topic_alert_temp.qos),
                (&n.topic_monitor.topic, n.topic_monitor.qos),
                (&n.topic_hub_setting_ok.topic, n.topic_hub_setting_ok.qos),
                (&n.topic_hub_firmware_ok.topic, n.topic_hub_firmware_ok.qos),
                (&n.topic_balance_mode_handshake.topic, n.topic_balance_mode_handshake.qos),
            ];

            if let Some((_, qos)) = pairs.iter().find(|(pattern, _)| topic_matches(pattern, topic_in)) {
                qos_topic = *qos;
            } else {
                // Si no coincide con ninguno de los patrones configurados, no hay un QoS para darle
                return None
            }

            Some(Topic::new(format!("iot/{net}/edge/{id}/{type_msg}"), qos_topic))
        } else {
            None
        }
    }
    
    /// Transforma un tópico remoto (proveniente del Servidor) en un tópico de destino para el Hub local.
    ///
    /// Esta función actúa como un **Router de Bajada** (Downlink). Filtra los mensajes de control
    /// enviados por el servidor y los traduce a los tópicos específicos que el Hub local
    /// está escuchando.
    ///
    /// # Lógica de Transformación
    ///
    /// A diferencia de la función de subida, esta función realiza un mapeo **explícito y estático**
    /// de comandos específicos.
    ///
    /// | Tipo de Comando | Tópico de Salida (hacia el Hub) |
    /// |-----------------|---------------------------------|
    /// | Red Activa/Inactiva | `iot/{red}/network_active` |
    /// | Nueva Configuración | `iot/{red}/new_setting_to_hub` |
    /// | Nuevo Firmware | `iot/{red}/new_firmware_to_hub` |
    ///
    /// # Proceso de Validación
    ///
    /// 1. Verifica la estructura base del tópico.
    /// 2. Busca la red en memoria.
    /// 3. Compara el tópico de entrada contra los tópicos de control específicos configurados en `Network`.
    ///
    /// # Retorno
    ///
    /// - `Some(Topic)`: Con el tópico traducido y el QoS configurado.
    /// - `None`: Si el mensaje no corresponde a un comando de control conocido.
    pub fn get_topic_to_send_msg_from_server(&self, topic_in: &str) -> Option<Topic> {

        let parts: Vec<&str> = topic_in.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let net = parts[1];
        let qos_topic : u8;

        if let Some(n) = self.networks.get(net) {
            if topic_matches(&n.topic_network.topic, topic_in) {
                qos_topic = n.topic_network.qos;
                Some(Topic::new(format!("iot/{net}/network_active"), qos_topic))
            } else if topic_matches(&n.topic_new_setting_to_hub.topic, topic_in) {
                qos_topic = n.topic_new_setting_to_hub.qos;
                Some(Topic::new(format!("iot/{net}/new_setting_to_hub"), qos_topic))
            } else if topic_matches(&n.topic_new_firmware.topic, topic_in) {
                qos_topic = n.topic_new_firmware.qos;
                Some(Topic::new(format!("iot/{net}/new_firmware_to_hub"), qos_topic))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn get_topic_to_send_msg_from_network(&self, topic_in: &str) -> Option<Topic> {

        let parts: Vec<&str> = topic_in.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let net = parts[1];
        let qos_topic : u8;

        if let Some(n) = self.networks.get(net) {
            if topic_matches(&n.topic_active_hub.topic, topic_in) {
                qos_topic = n.topic_active_hub.qos;
                Some(Topic::new(format!("iot/{net}/active_hub"), qos_topic))
            } else if topic_matches(&n.topic_delete_hub.topic, topic_in) {
                qos_topic = n.topic_delete_hub.qos;
                Some(Topic::new(format!("iot/{net}/delete_hub"), qos_topic))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn get_topic_to_send_firmware_ok(&self, topic_in: &str, id: &str) -> Option<Topic> {

        let parts: Vec<&str> = topic_in.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let net = parts[1];
        let qos_topic : u8;

        if let Some(n) = self.networks.get(net) {
            if topic_matches(&n.topic_new_firmware.topic, topic_in) {
                qos_topic = n.topic_hub_firmware_ok.qos;
                Some(Topic::new(format!("iot/{net}/edge/{id}/hub_firmware_ok"), qos_topic))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn cast_topic_from_network_to_delete(&self, topic_in: &str, id: &str) -> Option<Topic> {

        let parts: Vec<&str> = topic_in.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let net = parts[1];
        let qos_topic : u8;

        if let Some(n) = self.networks.get(net) {
            if topic_matches(&n.topic_network.topic, topic_in) {
                qos_topic = n.topic_delete_hub.qos;
                Some(Topic::new(format!("iot/{net}/edge/{id}/delete_hub"), qos_topic))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn cast_topic_from_network_to_active(&self, topic_in: &str) -> Option<Topic> {

        let parts: Vec<&str> = topic_in.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let net = parts[1];
        let qos_topic : u8;

        if let Some(n) = self.networks.get(net) {
            if topic_matches(&n.topic_new_firmware.topic, topic_in) {
                qos_topic = n.topic_active_hub.qos;
                Some(Topic::new(format!("iot/{net}/active_hub"), qos_topic))
            } else {
                None
            }
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
/// - `topic_new_setting_to_hub`: tópico donde se suscribe el Edge para recibir mensajes del servidor para configurar hubs.
/// - `topic_hub_setting_ok`: tópico donde se suscribe el Edge para recibir la confirmación de config aplicada de los hubs.
/// - `topic_new_firmware`: tópico donde se suscribe el Edge para recibir mensajes del servidor para actualizar el firmware de los hubs.
/// - `topic_hub_firmware_ok`: tópico donde se suscribe el Edge para recibir el handshake del hub de nuevo firmware listo.
/// - `topic_balance_mode_handshake`: tópico donde se suscribe el Edge para recibir mensaje de handshake de los nodos (en balance mode).
/// - `active`: variable que indica si la red actualmente está activa o inactiva.
#[derive(Debug, Clone)]
pub struct Network {
    pub id_network: String,
    pub name_network: String,
    pub topic_data: Topic,
    pub topic_alert_air: Topic,
    pub topic_alert_temp: Topic,
    pub topic_monitor: Topic,
    pub topic_hub_setting_ok: Topic,
    pub topic_hub_firmware_ok: Topic,
    pub topic_balance_mode_handshake: Topic,

    pub topic_setting: Topic,
    pub topic_delete_hub: Topic,
    pub topic_setting_ok: Topic,
    pub topic_active_hub: Topic,

    pub topic_hello_world: Topic,

    pub topic_network: Topic,
    pub topic_new_setting_to_hub: Topic,
    pub topic_new_firmware: Topic,
    pub active: bool,
}


impl Network {
    pub fn new(id_network: String, name_network: String, active: bool, system: &System) -> Self {
        let id_system = system.id_edge.clone();
        let t_data = format!("iot/{id_network}/hub/+/data");
        let t_alert_air = format!("iot/{id_network}/hub/+/alert_air");
        let t_alert_temp = format!("iot/{id_network}/hub/+/alert_temp");
        let t_monitor = format!("iot/{id_network}/hub/+/monitor");
        let t_network = format!("iot/{id_network}/edge/{id_system}/network");
        let t_new_setting_to_hub = format!("iot/{id_network}/edge/{id_system}/new_setting_to_hub");
        let t_hub_setting_ok = format!("iot/{id_network}/hub/+/hub_setting_ok");
        let t_new_firmware = format!("iot/{id_network}/edge/{id_system}/new_firmware");
        let t_hub_firmware_ok = format!("iot/{id_network}/hub/+/hub_firmware_ok");
        let t_balance_mode_handshake = format!("iot/{id_network}/hub/+/balance_mode_handshake");
        let t_setting = format!("iot/{id_network}/hub/+/setting");
        let t_delete_hub = format!("iot/{id_network}/edge/{id_system}/delete_hub");
        let t_setting_ok = format!("iot/{id_network}/edge/{id_system}/setting_ok");
        let t_hello_world = format!("iot/{id_network}/edge/{id_system}/hello_world");

        let t_active_hub = format!("iot/{id_network}/active_hub");

        Self {
            id_network,
            name_network,
            topic_data: Topic::new(t_data, 0),
            topic_alert_air: Topic::new(t_alert_air, 1),
            topic_alert_temp: Topic::new(t_alert_temp, 1),
            topic_monitor: Topic::new(t_monitor, 0),
            topic_network: Topic::new(t_network, 0),
            topic_new_setting_to_hub: Topic::new(t_new_setting_to_hub, 0),
            topic_hub_setting_ok: Topic::new(t_hub_setting_ok, 0),
            topic_new_firmware: Topic::new(t_new_firmware, 0),
            topic_hub_firmware_ok: Topic::new(t_hub_firmware_ok, 0),
            topic_balance_mode_handshake: Topic::new(t_balance_mode_handshake, 0),
            topic_setting: Topic::new(t_setting, 0),
            topic_delete_hub: Topic::new(t_delete_hub, 0),
            topic_setting_ok: Topic::new(t_setting_ok, 0),
            topic_active_hub: Topic::new(t_active_hub, 0),
            topic_hello_world: Topic::new(t_hello_world, 0),
            active
        }
    }
}



/// Modelo de datos plano para mapeo directo con SQLx (SQLite).
#[derive(Debug, FromRow, Deserialize, PartialEq, Clone)]
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

    pub fn new(id_network: String, name_network: String, active: bool) -> Self {
        Self { id_network, name_network, active }
    }
}


#[derive(Debug, PartialEq)]
pub enum NetworkAction {
    Delete,
    Update,
    Insert,
    Ignore,
}


#[derive(Debug, PartialEq, Clone)]
pub enum NetworkChanged {
    Insert(NetworkRow),
    Update(NetworkRow),
    Delete { id: String },
}


#[derive(Debug, PartialEq, Clone)]
pub enum HubChanged {
    Insert(HubRow),
    Update(HubRow),
    Delete(String),
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateNetwork { Changed, NotChanged }


#[derive(Debug, Clone, Default, Hash, Eq, PartialEq)]
pub struct Hub {
    pub id: String,
    pub device_name: String,
    pub energy_mode: u8,
}