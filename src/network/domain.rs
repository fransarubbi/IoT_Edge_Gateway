use sqlx::{FromRow};
use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use crate::system::domain::System;
use rand::seq::IteratorRandom;
use tokio::sync::mpsc;
use crate::context::domain::AppContext;
use crate::database::domain::DataServiceCommand;
use crate::message::domain::{HubMessage, Metadata, ServerMessage};
use crate::network::logic::{network_admin, network_dba};

pub enum NetworkServiceResponse {
    HubMessage(HubMessage),
    DataCommand(DataServiceCommand),
}


pub enum NetworkServiceCommand {
    HubMessage(HubMessage),
    ServerMessage(ServerMessage),
    Batch(Batch),
}


pub enum Batch {
    Network(Vec<NetworkRow>),
    Hub(Vec<HubRow>),
}


pub struct NetworkService {
    sender: mpsc::Sender<NetworkServiceResponse>,
    receiver: mpsc::Receiver<NetworkServiceCommand>,
    context: AppContext,
}


impl NetworkService {
    pub fn new(sender: mpsc::Sender<NetworkServiceResponse>,
               receiver: mpsc::Receiver<NetworkServiceCommand>,
               context: AppContext) -> Self {
        Self {
            sender,
            receiver,
            context
        }
    }

    pub async fn run(mut self) {

        let (tx_to_insert_network, rx_from_network) = mpsc::channel::<NetworkChanged>(50);
        let (tx_to_core, mut rx_response_admin) = mpsc::channel::<HubMessage>(50);
        let (tx_to_insert_hub, rx_from_network_hub) = mpsc::channel::<HubChanged>(50);
        let (tx_server_command, rx_from_server) = mpsc::channel::<ServerMessage>(50);
        let (tx_hub_command, rx_from_hub) = mpsc::channel::<HubMessage>(50);
        let (tx_dba_response, mut rx_response_dba) = mpsc::channel::<DataServiceCommand>(50);
        let (tx_command_batch, rx_batch) = mpsc::channel::<Batch>(50);

        tokio::spawn(network_admin(tx_to_insert_network,
                                   tx_to_core,
                                   tx_to_insert_hub,
                                   rx_from_server,
                                   rx_from_hub,
                                   rx_batch,
                                   self.context.clone()));

        tokio::spawn(network_dba(tx_dba_response,
                                 rx_from_network,
                                 rx_from_network_hub));

        loop {
            tokio::select! {
                Some(command) = self.receiver.recv() => {
                    match command {
                        NetworkServiceCommand::HubMessage(message) => {
                            if tx_hub_command.send(message).await.is_err() {
                                error!("Error: no se pudo enviar HubMessage");
                            }
                        },
                        NetworkServiceCommand::ServerMessage(message) => {
                            if tx_server_command.send(message).await.is_err() {
                                error!("Error: no se pudo enviar ServerMessage");
                            }
                        },
                        NetworkServiceCommand::Batch(batch) => {
                            match batch {
                                Batch::Network(_) => {
                                    if tx_command_batch.send(batch).await.is_err() {
                                        error!("Error: no se pudo enviar BatchNetwork");
                                    }
                                }
                                Batch::Hub(_) => {
                                    if tx_command_batch.send(batch).await.is_err() {
                                        error!("Error: no se pudo enviar BatchHub");
                                    }
                                }
                            }
                        }
                    }
                }
                Some(message) = rx_response_admin.recv() => {
                    if self.sender.send(NetworkServiceResponse::HubMessage(message)).await.is_err() {
                        error!("Error: no se pudo enviar HubMessage");
                    }
                }
                Some(data_command) = rx_response_dba.recv() => {
                    if self.sender.send(NetworkServiceResponse::DataCommand(data_command)).await.is_err() {
                        error!("Error: no se pudo enviar DataServiceCommand");
                    }
                }
            }
        }
    }
}


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

    /// Obtiene la cantidad total de hubs asociados al Edge.
    pub fn get_total_hubs(&self) -> u64 {
        let mut total_hubs = 0;
        for hub in self.hubs.values() {
            let total = hub.len() as u64;
            total_hubs += total
        }
        total_hubs
    }
    
    pub fn get_topic_to_send_msg_to_hub(&self, msg: &HubMessage) -> Option<Topic> {

        match msg {
            HubMessage::UpdateFirmware(firmware) => {
                let id_net = firmware.network.clone();
                if let Some(n) = self.networks.get(&id_net) {
                    Some(n.topic_new_firmware.clone())
                } else {
                    None
                }
            },
            HubMessage::FromServerSettings(settings) => {
                let id_net = settings.network.clone();
                if let Some(n) = self.networks.get(&id_net) {
                    Some(n.topic_new_setting.clone())
                } else {
                    None
                }
            },
            HubMessage::FromServerSettingsAck(settings_ack) => {
                let id_net = settings_ack.network.clone();
                if let Some(n) = self.networks.get(&id_net) {
                    Some(n.topic_setting_ok.clone())
                } else {
                    None
                }
            },
            HubMessage::DeleteHub(delete_hub) => {
                let id_net = delete_hub.network.clone();
                if let Some(n) = self.networks.get(&id_net) {
                    Some(n.topic_delete_hub.clone())
                } else { 
                    None 
                }
            },
            HubMessage::ActiveHub(active_hub) => {
                let id_net = active_hub.network.clone();
                if let Some(n) = self.networks.get(&id_net) {
                    Some(n.topic_active_hub.clone())
                } else {
                    None
                }
            },
            HubMessage::PingToHub(ping) => {
                let id_net = ping.network.clone();
                if let Some(n) = self.networks.get(&id_net) {
                    Some(n.topic_ping_ack.clone())
                } else {
                    None
                }
            }
            _ => None,
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

    // Tópicos de recepción
    pub topic_data: Topic,
    pub topic_alert_air: Topic,
    pub topic_alert_temp: Topic,
    pub topic_monitor: Topic,
    pub topic_hub_setting_ok: Topic,
    pub topic_hub_firmware_ok: Topic,
    pub topic_balance_mode_handshake: Topic,
    pub topic_setting: Topic,
    pub topic_ping: Topic,

    // Tópicos de envío
    pub topic_new_setting: Topic,
    pub topic_new_firmware: Topic,
    pub topic_setting_ok: Topic,
    pub topic_delete_hub: Topic,
    pub topic_active_hub: Topic,
    pub topic_ping_ack: Topic,

    pub active: bool,
}


impl Network {
    pub fn new(id_network: String, name_network: String, active: bool) -> Self {
        let t_data = format!("iot/{id_network}/hub/+/data");
        let t_alert_air = format!("iot/{id_network}/hub/+/alert_air");
        let t_alert_temp = format!("iot/{id_network}/hub/+/alert_temp");
        let t_monitor = format!("iot/{id_network}/hub/+/monitor");
        let t_hub_setting_ok = format!("iot/{id_network}/hub/+/hub_setting_ok");
        let t_hub_firmware_ok = format!("iot/{id_network}/hub/+/hub_firmware_ok");
        let t_balance_mode_handshake = format!("iot/{id_network}/hub/+/balance_mode_handshake");
        let t_setting = format!("iot/{id_network}/hub/+/setting");
        let t_new_setting = format!("iot/{id_network}/new_setting");
        let t_new_firmware = format!("iot/{id_network}/new_firmware");
        let t_setting_ok = format!("iot/{id_network}/new_setting_ok");
        let t_delete_hub = format!("iot/{id_network}/delete_hub");
        let t_active = format!("iot/{id_network}/active");
        let t_ping = format!("iot/{id_network}/hub/+/ping");
        let t_ping_ack = format!("iot/{id_network}/ping");

        Self {
            id_network,
            name_network,
            topic_data: Topic::new(t_data, 0),
            topic_alert_air: Topic::new(t_alert_air, 1),
            topic_alert_temp: Topic::new(t_alert_temp, 1),
            topic_monitor: Topic::new(t_monitor, 0),
            topic_hub_setting_ok: Topic::new(t_hub_setting_ok, 0),
            topic_hub_firmware_ok: Topic::new(t_hub_firmware_ok, 0),
            topic_balance_mode_handshake: Topic::new(t_balance_mode_handshake, 0),
            topic_setting: Topic::new(t_setting, 0),
            topic_new_setting: Topic::new(t_new_setting, 0),
            topic_new_firmware: Topic::new(t_new_firmware, 0),
            topic_setting_ok: Topic::new(t_setting_ok, 0),
            topic_delete_hub: Topic::new(t_delete_hub, 0),
            topic_active_hub: Topic::new(t_active, 0),
            topic_ping: Topic::new(t_ping, 1),
            topic_ping_ack: Topic::new(t_ping_ack, 1),
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
    pub fn cast_to_network(self) -> Network {
        Network::new(self.id_network, self.name_network, self.active)
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


#[derive(Debug, Clone, Default, Hash, Eq, PartialEq)]
pub struct Hub {
    pub id: String,
    pub device_name: String,
    pub energy_mode: u32,
}


#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow, Hash)]
pub struct HubRow {
    #[sqlx(flatten)]
    pub metadata: Metadata,
    pub network_id: String,
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub mqtt_uri: String,
    pub device_name: String,
    pub sample: u32,
    pub energy_mode: u32,
}


impl HubRow {
    pub fn cast_to_hub(self) -> Hub {
        let mut hub = Hub::default();
        hub.id = self.metadata.sender_user_id;
        hub.device_name = self.device_name;
        hub.energy_mode = self.energy_mode;
        hub
    }
}