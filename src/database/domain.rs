//! Definiciones de dominio para la capa de persistencia y base de datos.
//!
//! Este módulo contiene las estructuras de datos, enumeraciones y lógica auxiliar
//! necesaria para:
//! 1. Representar entidades de la base de datos en memoria (Buffers).
//! 2. Definir rutas de mensajes (Servidor vs. Base de Datos).
//! 3. Gestionar la máquina de estados para la sincronización de datos pendientes.


use tokio::sync::{mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use crate::config::sqlite::BATCH_SIZE;
use crate::database::logic::{dba_get_task, dba_insert_task, dba_remove_task};
use crate::database::repository::Repository;
use crate::message::domain::{AlertAir, AlertTh, HubMessage, Measurement, Monitor};
use crate::network::domain::{HubRow, NetworkRow};
use crate::system::domain::InternalEvent;


/// Comandos que recibe el servicio de base de datos
/// desde fuera. Son todas las operaciones que debe realizar.
pub enum DataServiceCommand {
    Hub(HubMessage),
    Internal(InternalEvent),
    NewEpoch(u32),
    GetEpoch,
    NewHub(HubRow),
    DeleteHub(String),
    UpdateHub(HubRow),
    NewNetwork(NetworkRow),
    DeleteNetwork(String),
    UpdateNetwork(NetworkRow),
    DeleteAllHubByNetwork(String),
    GetTotalOfNetworks,
}


/// Wrapper para las posibles respuestas y datos
/// enviados por el servicio de base de datos.
pub enum DataServiceResponse {
    Batch(TableDataVector),
    BatchNetwork(Vec<NetworkRow>),
    BatchHub(Vec<HubRow>),
    Epoch(u32),
    ErrorEpoch,
    NoNetworks,
    ThereAreNetworks,
    NetworksUpdated,
}


/// Comandos internos para la tarea get.
pub enum DataCommandGet {
    GetEpoch,
    GetTotalOfNetworks,
}


/// Comandos internos para la tarea delete.
pub enum DataCommandDelete {
    DeleteNetwork(String),
    DeleteAllHubByNetwork(String),
    DeleteHub(String),
}


/// Comandos internos para la tarea insert.
pub enum DataCommandInsert {
    InsertNetwork(NetworkRow),
    UpdateNetwork(NetworkRow),
    NewEpoch(u32),
    InsertHub(HubRow),
    UpdateHub(HubRow),
}


pub struct DataService {
    sender: mpsc::Sender<DataServiceResponse>,
    receiver: mpsc::Receiver<DataServiceCommand>,
    repo: Repository,
}


impl DataService {
    pub fn new(sender: mpsc::Sender<DataServiceResponse>,
               receiver: mpsc::Receiver<DataServiceCommand>,
               repo: Repository) -> Self {
        Self {
            sender,
            receiver,
            repo,
        }
    }

    pub async fn run(mut self, shutdown: CancellationToken) {

        let (tx_to_core, mut rx_response_from_insert) = mpsc::channel::<DataServiceResponse>(50);
        let (tx_response, mut rx_response) = mpsc::channel::<DataServiceResponse>(50);
        let (tx_msg, rx_msg) = mpsc::channel::<HubMessage>(50);
        let (tx_command_insert, rx_command_insert) = mpsc::channel::<DataCommandInsert>(50);
        let (tx_internal, rx_internal) = mpsc::channel::<InternalEvent>(50);
        let (tx_command_get, rx_command_get) = mpsc::channel::<DataCommandGet>(50);
        let (tx_command_delete, rx_command_delete) = mpsc::channel::<DataCommandDelete>(50);
        let (tx_response_from_dba, mut rx_response_from_remove) = mpsc::channel::<DataServiceResponse>(50);

        tokio::spawn(dba_insert_task(
                        tx_to_core,
                        rx_msg,
                        rx_command_insert,
                        self.repo.clone(),
                        shutdown.clone()));

        tokio::spawn(dba_get_task(
                        tx_response,
                        rx_internal,
                        rx_command_get,
                        self.repo.clone(),
                        shutdown.clone()));

        tokio::spawn(dba_remove_task(
                        tx_response_from_dba,
                        rx_command_delete,
                        self.repo.clone(),
                        shutdown.clone()));

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("Info: shutdown recibido DataService");
                    break;
                }
                Some(cmd) = self.receiver.recv() => {
                    match cmd {
                        DataServiceCommand::Hub(hub_msg) => {
                            if tx_msg.send(hub_msg).await.is_err() {
                                error!("Error: no se pudo enviar HubMessage a dba_insert_task");
                            }
                        },
                        DataServiceCommand::Internal(internal_event) => {
                            if tx_internal.send(internal_event).await.is_err() {
                                error!("Error: no se pudo enviar InternalEvent");
                            }
                        },
                        DataServiceCommand::NewEpoch(epoch) => {
                            if tx_command_insert.send(DataCommandInsert::NewEpoch(epoch)).await.is_err() {
                                error!("Error: no se pudo enviar comando NewEpoch a dba_insert_task");
                            }
                        },
                        DataServiceCommand::GetEpoch => {
                            if tx_command_get.send(DataCommandGet::GetEpoch).await.is_err() {
                                error!("Error: no se pudo enviar comando GetEpoch a dba_get_task");
                            }
                        },
                        DataServiceCommand::NewHub(row) => {
                            if tx_command_insert.send(DataCommandInsert::InsertHub(row)).await.is_err() {
                                error!("Error: no se pudo enviar comando InsertHub a dba_insert_task");
                            }
                        },
                        DataServiceCommand::DeleteHub(hub_id) => {
                            if tx_command_delete.send(DataCommandDelete::DeleteHub(hub_id)).await.is_err() {
                                error!("Error: no se pudo enviar comando DeleteHub a dba_remove_task");
                            }
                        },
                        DataServiceCommand::UpdateHub(hub) => {
                            if tx_command_insert.send(DataCommandInsert::UpdateHub(hub)).await.is_err() {
                                error!("Error: no se pudo enviar comando UpdateHub a dba_insert_task");
                            }
                        },
                        DataServiceCommand::NewNetwork(network) => {
                            if tx_command_insert.send(DataCommandInsert::InsertNetwork(network)).await.is_err() {
                                error!("Error: no se pudo enviar comando InsertNetwork a dba_insert_task");
                            }
                        },
                        DataServiceCommand::DeleteNetwork(id_network) => {
                            if tx_command_delete.send(DataCommandDelete::DeleteNetwork(id_network)).await.is_err() {
                                error!("Error: no se pudo enviar comando DeleteNetwork a dba_remove_task");
                            }
                        },
                        DataServiceCommand::UpdateNetwork(network) => {
                            if tx_command_insert.send(DataCommandInsert::UpdateNetwork(network)).await.is_err() {
                                error!("Error: no se pudo enviar comando UpdateNetwork a dba_insert_task");
                            }
                        },
                        DataServiceCommand::DeleteAllHubByNetwork(id) => {
                            if tx_command_delete.send(DataCommandDelete::DeleteAllHubByNetwork(id)).await.is_err() {
                                error!("Error: no se pudo enviar comando DeleteAllHubByNetwork a dba_remove_task");
                            }
                        },
                        DataServiceCommand::GetTotalOfNetworks => {
                            if tx_command_get.send(DataCommandGet::GetTotalOfNetworks).await.is_err() {
                                error!("Error: no se pudo enviar comando GetTotalOfNetworks a dba_get_task");
                            }
                        }
                    }
                }

                Some(response) = rx_response.recv() => {
                    if self.sender.send(response).await.is_err() {
                        error!("Error: no se pudo enviar DataServiceResponse al Core");
                    }
                }

                Some(response) = rx_response_from_remove.recv() => {
                    if self.sender.send(response).await.is_err() {
                        error!("Error: no se pudo enviar NoNetworks al Core");
                    }
                }

                Some(response) = rx_response_from_insert.recv() => {
                    if self.sender.send(response).await.is_err() {
                        error!("Error: no se pudo enviar ThereAreNetworks al Core");
                    }
                }
            }
        }
    }
}


/// Estructura que agrupa un lote de datos de un tipo específico junto con su metadato de origen.
/// Se utiliza para mover batches desde la DB hacia el sistema de mensajería.
#[derive(Clone, Debug, Default)]
pub struct TableDataVector {
    pub measurement: Vec<Measurement>,
    pub alert_air: Vec<AlertAir>,
    pub alert_th: Vec<AlertTh>,
    pub monitor: Vec<Monitor>,
}


impl TableDataVector {

    /// Crea un nuevo contenedor con la capacidad pre-reservada.
    ///
    /// Inicializa los vectores internos utilizando `Vec::with_capacity(BATCH_SIZE)`.
    /// Esto evita realocaciones dinámicas de memoria mientras se llena el buffer,
    /// mejorando el rendimiento de inserción.
    pub fn new() -> Self {
        Self {
            measurement: Vec::with_capacity(BATCH_SIZE),
            alert_air: Vec::with_capacity(BATCH_SIZE),
            alert_th: Vec::with_capacity(BATCH_SIZE),
            monitor: Vec::with_capacity(BATCH_SIZE),
        }
    }
    
    /// Constructor con parámetros para hacer pop batch.
    pub fn new_pop(measurement: Vec<Measurement>, 
                   alert_air: Vec<AlertAir>, 
                   alert_th: Vec<AlertTh>,
                   monitor: Vec<Monitor>) -> Self {
        Self {
            measurement,
            alert_air,
            alert_th,
            monitor,
        }
    }
    
    /// Retorna true si todos los vectores están vacíos.
    pub fn is_empty(&self) -> bool { 
        self.measurement.is_empty() && 
            self.alert_air.is_empty() && 
            self.alert_th.is_empty() && 
            self.monitor.is_empty()
    }

    /// Verifica si alguno de los buffers internos ha alcanzado su capacidad máxima.
    ///
    /// Este método se utiliza como disparador (Trigger) para realizar el volcado (flush)
    /// a la base de datos.
    ///
    /// # Retorno
    /// * `true`: Al menos uno de los vectores tiene longitud igual a `BATCH_SIZE`.
    /// * `false`: Todos los vectores tienen espacio disponible.
    pub fn is_some_vector_full(&self) -> bool {
        self.is_measurement_full() || self.is_alert_air_full() ||
            self.is_alert_th_full() || self.is_monitor_full()
    }

    fn is_measurement_full(&self) -> bool {
        self.measurement.len() == BATCH_SIZE
    }
    fn is_monitor_full(&self) -> bool {
        self.monitor.len() == BATCH_SIZE
    }
    fn is_alert_air_full(&self) -> bool {
        self.alert_air.len() == BATCH_SIZE
    }
    fn is_alert_th_full(&self) -> bool {
        self.alert_th.len() == BATCH_SIZE
    }

    /// Reinicia los buffers sin liberar la memoria asignada.
    ///
    /// Establece la longitud de todos los vectores a 0, pero mantiene la capacidad
    /// reservada en el Heap. Esto permite reutilizar la estructura en el siguiente
    /// ciclo de acumulación sin costo de asignación de memoria.
    pub fn clear(&mut self) {
        self.measurement.clear();
        self.alert_air.clear();
        self.alert_th.clear();
        self.monitor.clear();
    }
}