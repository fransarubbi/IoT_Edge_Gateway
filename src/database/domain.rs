use tokio::sync::{mpsc, watch};
use crate::message::domain::{AlertAir, AlertTh, DataRequest, Measurement, MessageFromHub, MessageToServer, Monitor};
use crate::message::domain::MessageToServer::HubToServer;
use crate::message::domain_for_table::{AlertAirRow, AlertThRow, MeasurementRow, MonitorRow};

pub enum Route {
    ToServer(MessageToServer),
    ToDatabase(MessageFromHub),
}


#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Table {
    Measurement,
    Monitor,
    AlertAir,
    AlertTemp,
    Error,
}


impl Table {
    pub fn table_name(&self) -> &'static str {
        match self {
            Table::Measurement => "measurement",
            Table::Monitor => "monitor",
            Table::AlertAir => "alert_air",
            Table::AlertTemp => "alert_temp",
            _ => "error",
        }
    }
    pub fn all() -> &'static [Table] {
        &[
            Table::Measurement,
            Table::Monitor,
            Table::AlertAir,
            Table::AlertTemp
        ]
    }
}


#[derive(Clone, Debug, PartialEq)]
pub enum TableData {
    Measurement(Measurement),
    Monitor(Monitor),
    AlertAir(AlertAir),
    AlertTemp(AlertTh),
}


#[derive(Clone, Debug)]
pub struct TableDataVector {
    pub topic_where_arrive: String,
    pub vec: TableDataVectorTypes,
}


impl TableDataVector {
    pub fn is_empty(&self) -> bool {
        match &self.vec {
            TableDataVectorTypes::Measurement(v) => v.is_empty(),
            TableDataVectorTypes::Monitor(v) => v.is_empty(),
            TableDataVectorTypes::AlertAir(v) => v.is_empty(),
            TableDataVectorTypes::AlertTemp(v) => v.is_empty(),
        }
    }
}


#[derive(Clone, Debug)]
pub enum TableDataVectorTypes {
    Measurement(Vec<MeasurementRow>),
    Monitor(Vec<MonitorRow>),
    AlertAir(Vec<AlertAirRow>),
    AlertTemp(Vec<AlertThRow>),
}


impl TableDataVector {
    pub fn new(topic_where_arrive: String, vec: TableDataVectorTypes) -> Self {
        Self { topic_where_arrive, vec }
    }

    pub fn table_type(&self) -> Table {
        match self.vec {
            TableDataVectorTypes::Measurement(_) => Table::Measurement,
            TableDataVectorTypes::Monitor(_) => Table::Monitor,
            TableDataVectorTypes::AlertAir(_) => Table::AlertAir,
            TableDataVectorTypes::AlertTemp(_) => Table::AlertTemp,
        }
    }
}


#[derive(Default, Debug)] 
pub struct Vectors {
    pub measurements: Vec<MeasurementRow>,
    pub monitors: Vec<MonitorRow>,
    pub alert_airs: Vec<AlertAirRow>,
    pub alert_temps: Vec<AlertThRow>,
}


impl Vectors {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            measurements: Vec::with_capacity(cap),
            monitors: Vec::with_capacity(cap),
            alert_airs: Vec::with_capacity(cap),
            alert_temps: Vec::with_capacity(cap),
        }
    }
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


async fn send_ignore<T>(tx: &mpsc::Sender<T>, msg: T) {
    let _ = tx.send(msg).await;
}


pub fn route_message(msg: MessageFromHub,
                    server_connected: bool,
                    ) -> Route {
    if server_connected {
        Route::ToServer(HubToServer(msg))
    } else {
        Route::ToDatabase(msg)
    }
}


#[derive(Debug, Clone)]
pub enum StateFlag {
    Init,
    Measurement,
    Monitor,
    AlertAir,
    AlertTh,
}


impl StateFlag {
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
                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                    }
                }
                *self = StateFlag::Init;
            },
        }
    }
}


async fn check_flag(flag: bool, tx: &watch::Sender<DataRequest>) -> bool {
    if flag {
        if tx.send(DataRequest::Get).is_err() {
            log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
        }
        return true;
    }
    false
}