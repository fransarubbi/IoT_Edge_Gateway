//! Módulo de Gestión de Redes y Topología IoT.
//!
//! Este módulo actúa como el **Plano de Control** del Edge Gateway.
//! Es responsable de mantener el estado y la configuración de todas las redes lógicas
//! y los dispositivos físicos (Hubs) asociados a este Edge.
//!
//! # Arquitectura
//!
//! Se divide en tres componentes principales:
//! 1. **[`NetworkService`]:** El actor asíncrono que orquesta los flujos de mensajes.
//! 2. **[`NetworkManager`]:** La caché en memoria que almacena topologías y pre-calcula tópicos MQTT.
//! 3. **Modelos de Datos (`NetworkRow`, `HubRow`):** Representaciones planas para la persistencia en SQLite.
//!
//! # Estrategia de Caché
//!
//! Para evitar que cada mensaje MQTT entrante requiera una consulta a la base de datos,
//! el `NetworkManager` mantiene una copia en memoria (`HashMap`) de las redes y hubs activos,
//! acelerando drásticamente el enrutamiento de mensajes.


use sqlx::{FromRow};
use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use crate::system::domain::System;
use rand::seq::IteratorRandom;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use crate::context::domain::AppContext;
use crate::database::domain::DataServiceCommand;
use crate::message::domain::{HubMessage, Metadata, ServerMessage};
use crate::network::logic::{network_admin, network_dba};


/// Respuestas emitidas por el servicio de red hacia el Core o FSM.
pub enum NetworkServiceResponse {
    /// Mensaje de negocio procesado que debe ser enrutado a un Hub.
    HubMessage(HubMessage),
    /// Comando de persistencia dirigido a la base de datos.
    DataCommand(DataServiceCommand),
    /// Señal de sincronización indicando que las redes fueron cargadas en memoria.
    NetworksReady,
}


/// Comandos entrantes que el servicio de red debe procesar.
pub enum NetworkServiceCommand {
    /// Mensaje proveniente de un Hub local.
    HubMessage(HubMessage),
    /// Comando o configuración proveniente del servidor (nube).
    ServerMessage(ServerMessage),
    /// Lote de datos de configuración recuperados de la base de datos local.
    Batch(Batch),
}


/// Lotes de entidades para carga inicial o sincronización masiva.
pub enum Batch {
    /// Lista plana de redes proveniente de la DB.
    Network(Vec<NetworkRow>),
    /// Lista plana de Hubs proveniente de la DB.
    Hub(Vec<HubRow>),
}


/// Actor principal que administra el subsistema de redes.
///
/// Orquesta dos sub-tareas:
/// - `network_admin`: Aplica reglas de negocio y actualiza la caché en memoria.
/// - `network_dba`: Gestiona la persistencia de los cambios de red en la base de datos.
pub struct NetworkService {
    /// Canal para emitir respuestas al exterior del módulo.
    sender: mpsc::Sender<NetworkServiceResponse>,
    /// Canal para recibir comandos del exterior.
    receiver: mpsc::Receiver<NetworkServiceCommand>,
    /// Contexto global de la aplicación.
    context: AppContext,
}


impl NetworkService {

    /// Crea una nueva instancia del servicio de red.
    pub fn new(sender: mpsc::Sender<NetworkServiceResponse>,
               receiver: mpsc::Receiver<NetworkServiceCommand>,
               context: AppContext) -> Self {
        Self {
            sender,
            receiver,
            context
        }
    }

    /// Ejecuta el bucle principal del servicio y lanza las tareas subordinadas.
    ///
    /// Crea la topología interna de canales para aislar la lógica de negocio (`admin`)
    /// de la lógica de persistencia (`dba`).
    pub async fn run(mut self, shutdown: CancellationToken) {

        let (tx_to_insert_network, rx_from_network) = mpsc::channel::<NetworkChanged>(50);
        let (tx_to_core, mut rx_response_admin) = mpsc::channel::<NetworkServiceResponse>(50);
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
                                   self.context.clone(),
                                   shutdown.clone()));

        tokio::spawn(network_dba(tx_dba_response,
                                 rx_from_network,
                                 rx_from_network_hub,
                                 shutdown.clone()));

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("Info: shutdown recibido NetworkService");
                    break;
                }
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
                Some(response) = rx_response_admin.recv() => {
                    if self.sender.send(response).await.is_err() {
                        error!("Error: no se pudo enviar NetworkServiceResponse");
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


/// Gestor en memoria de las configuraciones de redes, hubs y tópicos del sistema.
///
/// Este struct actúa como una caché de lectura rápida (O(1)) para evitar consultar
/// la base de datos cada vez que llega un mensaje o comando.
///
///
///
/// # Responsabilidades
/// - Almacenar la configuración de cada red y sus Hubs asociados.
/// - Resolver dinámicamente el tópico MQTT de destino según el tipo de mensaje.
/// - Administrar los tópicos globales del sistema (Handshake, State, Heartbeat).
#[derive(Debug, Clone)]
pub struct NetworkManager {
    /// Mapa de redes activas, indexado por `id_network`.
    pub networks: HashMap<String, Network>,
    /// Mapa de Hubs asociados a cada red. Key: `id_network`, Value: Conjunto de `Hub`s.
    pub hubs: HashMap<String, HashSet<Hub>>,

    // Tópicos globales de control
    pub topic_handshake: Topic,
    pub topic_state: Topic,
    pub topic_heartbeat: Topic,
}


impl NetworkManager {
    /// Crea una nueva instancia vacía del gestor.
    /// Los tópicos de handshake y state son globales, independientes de la red.
    /// Por ende tienen un path definido. Siempre es `iot/id_edge/handshake` o `iot/id_edge/state`
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

    /// Resuelve el tópico MQTT específico de salida basándose en el tipo de mensaje a enviar al Hub.
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


/// Configuración operativa completa de una Red IoT lógica.
///
/// Pre-calcula y almacena estáticamente todos los paths MQTT vinculados a su ID.
/// Esto evita tener que construir strings `format!()` repetitivamente durante la ejecución.
#[derive(Debug, Clone)]
pub struct Network {
    pub id_network: String,

    // ================= Tópicos Suscritos (Inbound) =================
    pub topic_data: Topic,
    pub topic_alert_air: Topic,
    pub topic_alert_temp: Topic,
    pub topic_monitor: Topic,
    pub topic_hub_setting_ok: Topic,
    pub topic_hub_firmware_ok: Topic,
    pub topic_balance_mode_handshake: Topic,
    pub topic_setting: Topic,
    pub topic_ping: Topic,

    // ================= Tópicos Publicados (Outbound) =================
    pub topic_new_setting: Topic,
    pub topic_new_firmware: Topic,
    pub topic_setting_ok: Topic,
    pub topic_delete_hub: Topic,
    pub topic_active_hub: Topic,
    pub topic_ping_ack: Topic,

    /// Indica si la red está en procesamiento activo o pausado.
    pub active: bool,
}


impl Network {

    /// Instancia una nueva red generando dinámicamente todos sus tópicos.
    /// Utiliza wildcards `+` en los tópicos entrantes para capturar eventos de cualquier hub en la red.
    pub fn new(id_network: String, active: bool) -> Self {
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


/// DTO (Data Transfer Object) para mapear redes planas desde la tabla SQL.
#[derive(Debug, FromRow, Deserialize, PartialEq, Clone)]
pub struct NetworkRow {
    pub id_network: String,
    pub active: bool,
}


/// Convierte una fila plana de base de datos (`NetworkRow`) a la estructura jerárquica (`Network`).
impl NetworkRow {
    pub fn cast_to_network(self) -> Network {
        Network::new(self.id_network, self.active)
    }

    pub fn new(id_network: String, active: bool) -> Self {
        Self { id_network, active }
    }
}


/// Acciones operacionales posibles sobre una red para notificar al DBA.
#[derive(Debug, PartialEq)]
pub enum NetworkAction {
    Delete,
    Update,
    Insert,
    Ignore,
}


/// Eventos de mutación de estado en redes para sincronizar la BD.
#[derive(Debug, PartialEq, Clone)]
pub enum NetworkChanged {
    Insert(NetworkRow),
    Update(NetworkRow),
    Delete { id: String },
}


/// Eventos de mutación de estado en Hubs para sincronizar la BD.
#[derive(Debug, PartialEq, Clone)]
pub enum HubChanged {
    Insert(HubRow),
    Update(HubRow),
    Delete(String),
}


/// Representación liviana en memoria caché de un dispositivo nodo/Hub.
#[derive(Debug, Clone, Default, Hash, Eq, PartialEq)]
pub struct Hub {
    pub id: String,
    pub device_name: String,
    pub energy_mode: u32,
}


/// DTO (Data Transfer Object) para mapear Hubs físicos desde la tabla SQL.
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