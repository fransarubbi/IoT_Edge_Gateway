use tokio::sync::{mpsc, watch};
use crate::message::msg_type::{AlertAir, AlertTh, DataRequest, Measurement, MessageFromHub, MessageToServer, Monitor};
use crate::message::msg_type::MessageToServer::HubToServer;


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
}


impl Table {
    pub fn table_name(&self) -> &'static str {
        match self {
            Table::Measurement => "measurement",
            Table::Monitor => "monitor",
            Table::AlertAir => "alert_air",
            Table::AlertTemp => "alert_temp",
        }
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
pub enum TableDataVector {
    Measurement(Vec<Measurement>),
    Monitor(Vec<Monitor>),
    AlertAir(Vec<AlertAir>),
    AlertTemp(Vec<AlertTh>),
}


impl TableDataVector {
    pub fn table_type(&self) -> Table {
        match self {
            TableDataVector::Measurement(_) => Table::Measurement,
            TableDataVector::Monitor(_) => Table::Monitor,
            TableDataVector::AlertAir(_) => Table::AlertAir,
            TableDataVector::AlertTemp(_) => Table::AlertTemp,
        }
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