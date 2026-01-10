//! Definiciones de dominio para la capa de persistencia y base de datos.
//!
//! Este módulo contiene las estructuras de datos, enumeraciones y lógica auxiliar
//! necesaria para:
//! 1. Representar entidades de la base de datos en memoria (Buffers).
//! 2. Definir rutas de mensajes (Servidor vs. Base de Datos).
//! 3. Gestionar la máquina de estados para la sincronización de datos pendientes.


use tokio::sync::{mpsc, watch};
use tracing::error;
use crate::message::domain_for_table::{AlertAirRow, AlertThRow, MeasurementRow, MonitorRow};
use crate::network::domain::Topic;


/// Enumeración de las tablas existentes en la base de datos SQLite para
/// persistencia de datos.
/// Utilizada para operaciones genéricas y mapeo de tópicos.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Table {
    Measurement,
    Monitor,
    AlertAir,
    AlertTemp,
    Error,
}


impl Table {
    /// Devuelve el nombre exacto de la tabla en SQL.
    pub fn table_name(&self) -> &'static str {
        match self {
            Table::Measurement => "measurement",
            Table::Monitor => "monitor",
            Table::AlertAir => "alert_air",
            Table::AlertTemp => "alert_temp",
            _ => "error",
        }
    }

    /// Retorna una lista estática de todas las tablas válidas.
    pub fn all() -> &'static [Table] {
        &[
            Table::Measurement,
            Table::Monitor,
            Table::AlertAir,
            Table::AlertTemp
        ]
    }
}


/// Estructura que agrupa un lote de datos de un tipo específico junto con su metadato de origen.
/// Se utiliza para mover batches desde la DB hacia el sistema de mensajería.
#[derive(Clone, Debug)]
pub struct TableDataVector {
    pub topic_where_arrive: String,
    pub vec: TableDataVectorTypes,
}


impl TableDataVector {
    /// Verifica si el vector interno no contiene elementos.
    pub fn is_empty(&self) -> bool {
        match &self.vec {
            TableDataVectorTypes::Measurement(v) => v.is_empty(),
            TableDataVectorTypes::Monitor(v) => v.is_empty(),
            TableDataVectorTypes::AlertAir(v) => v.is_empty(),
            TableDataVectorTypes::AlertTemp(v) => v.is_empty(),
        }
    }

    /// Crea una nueva instancia.
    pub fn new(topic_where_arrive: String, vec: TableDataVectorTypes) -> Self {
        Self { topic_where_arrive, vec }
    }

    /// Devuelve el tipo de tabla (`Table`) que corresponde a los datos contenidos.
    pub fn table_type(&self) -> Table {
        match self.vec {
            TableDataVectorTypes::Measurement(_) => Table::Measurement,
            TableDataVectorTypes::Monitor(_) => Table::Monitor,
            TableDataVectorTypes::AlertAir(_) => Table::AlertAir,
            TableDataVectorTypes::AlertTemp(_) => Table::AlertTemp,
        }
    }
}


/// Contenedor polimórfico para **vectores de filas** de base de datos.
/// Cada variante contiene un `Vec<T>` donde T es un struct `*Row`.
#[derive(Clone, Debug)]
pub enum TableDataVectorTypes {
    Measurement(Vec<MeasurementRow>),
    Monitor(Vec<MonitorRow>),
    AlertAir(Vec<AlertAirRow>),
    AlertTemp(Vec<AlertThRow>),
}


/// Buffer en memoria para acumular datos antes de insertarlos en la base de datos.
///
/// Contiene un vector separado para cada tipo de dato posible. Se utiliza en `dba_insert_task`
/// para realizar inserciones por lotes.
#[derive(Default, Debug)] 
pub struct Vectors {
    pub measurements: Vec<MeasurementRow>,
    pub monitors: Vec<MonitorRow>,
    pub alert_airs: Vec<AlertAirRow>,
    pub alert_temps: Vec<AlertThRow>,
}


impl Vectors {
    /// Inicializa los vectores internos reservando la capacidad especificada.
    /// Optimiza el rendimiento evitando re-allocations constantes.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            measurements: Vec::with_capacity(cap),
            monitors: Vec::with_capacity(cap),
            alert_airs: Vec::with_capacity(cap),
            alert_temps: Vec::with_capacity(cap),
        }
    }

    /// Verifica si **cualquiera** de los vectores internos ha alcanzado o superado la capacidad límite.
    /// Esto se usa como disparador (trigger) para guardar en disco.
    pub fn is_full(&self, cap: usize) -> bool {
        if self.measurements.len() >= cap {
            return true;
        }
        if self.monitors.len() >= cap {
            return true;
        }
        if self.alert_airs.len() >= cap {
            return true;
        }
        if self.alert_temps.len() >= cap {
            return true;
        }
        false
    }

    /// Verifica si **alguno** de los vectores está vacío.
    ///
    /// La lógica es: "si tengo al menos un vector vacío, retorno true".
    pub fn is_empty(&self) -> bool {
        if self.measurements.is_empty() {
            return true;
        }
        if self.monitors.is_empty() {
            return true;
        }
        if self.alert_airs.is_empty() {
            return true;
        }
        if self.alert_temps.is_empty() {
            return true;
        }
        false
    }
}


/// Función auxiliar asíncrona "Fire-and-Forget".
/// Envía un mensaje por un canal y descarta el resultado (éxito o error).
async fn send_ignore<T>(tx: &mpsc::Sender<T>, msg: T) {
    let _ = tx.send(msg).await;
}


/// Máquina de estados para el proceso de sincronización de datos pendientes.
///
/// Controla el orden en que se verifican las tablas (`Measurement` -> `Monitor` -> `Alertas`)
/// para extraer datos cuando se recupera la conexión.
#[derive(Debug, Clone)]
pub enum StateFlag {
    Init,
    Measurement,
    Monitor,
    AlertAir,
    AlertTh,
}


impl StateFlag {
    /// Avanza la máquina de estados.
    ///
    /// Si `flag` es true (hay datos en la tabla actual), envía una solicitud `Get` y se mantiene en el estado.
    /// Si `flag` es false (tabla vacía), avanza al siguiente estado/tabla.
    pub async fn update_state(&mut self, flag: bool, tx: &watch::Sender<DataRequest>) {
        match self {
            StateFlag::Init => {
                if !check_flag(flag, &tx).await {
                    *self = StateFlag::Measurement;
                }
            },
            StateFlag::Measurement => {
                if !check_flag(flag, &tx).await {
                    *self = StateFlag::Monitor;
                }
            },
            StateFlag::Monitor => {
                if !check_flag(flag, &tx).await {
                    *self = StateFlag::AlertAir;
                }
            }
            StateFlag::AlertAir => {
                if !check_flag(flag, &tx).await {
                    *self = StateFlag::AlertTh;
                }
            }
            StateFlag::AlertTh => {
                if !check_flag(flag, &tx).await {
                    if tx.send(DataRequest::NotGet).is_err() {
                        error!("Error: Receptor no disponible, mensaje descartado");
                    }
                }
                *self = StateFlag::Init;
            },
        }
    }
}


/// Auxiliar para verificar el flag de existencia de datos.
/// Si `flag` es true, solicita datos (`Get`) al canal.
async fn check_flag(flag: bool, tx: &watch::Sender<DataRequest>) -> bool {
    if flag {
        if tx.send(DataRequest::Get).is_err() {
            error!("Error: Receptor no disponible, mensaje descartado");
        }
        return true;
    }
    false
}


/// Representación auxiliar ligera de una Red para operaciones de lectura.
///
/// Se utiliza para tomar una "instantánea" (snapshot) de la configuración de red
/// necesaria para consultar la base de datos sin bloquear el `NetworkManager` principal.
#[derive(Debug, Clone)]
pub struct NetworkAux {
    pub id_network: String,
    pub topic_data: Topic,
    pub topic_alert_air: Topic,
    pub topic_alert_temp: Topic,
    pub topic_monitor: Topic,
}


impl NetworkAux {
    /// Crea una nueva instancia auxiliar de red.
    pub fn new(id_network: String,
               topic_data: Topic,
               topic_alert_air: Topic,
               topic_alert_temp: Topic,
               topic_monitor: Topic) -> Self {
        Self {
            id_network,
            topic_data,
            topic_alert_air,
            topic_alert_temp,
            topic_monitor
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataRequest { Get, NotGet }