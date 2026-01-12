use serde::{Deserialize, Serialize};
use crate::mqtt::domain::PayloadTopic;

#[derive(PartialEq, Clone)]
pub enum InternalEvent {
    ServerConnected,
    ServerDisconnected,
    LocalConnected,
    LocalDisconnected,
    IncomingMessage(PayloadTopic),
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubStateBalanceMode {
    InitBalanceMode,
    InHandshake,
    Quorum(SubStateQuorum),
    Phase(SubStatePhase),
    OutHandshake,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubStateQuorum {
    CheckQuorum,
    RepeatHandshake,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubStatePhase {
    Alert,
    Data,
    Monitor,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum State {
    BalanceMode(SubStateBalanceMode),
    Normal,
    SafeMode,
}


#[derive(Debug, Clone, PartialEq)]
pub enum EventSystem {
    EventServerConnected,
    EventServerDisconnected,
    EventLocalBrokerConnected,
    EventLocalBrokerDisconnected,
    //EventMessage(Message),
    EventSystemOk,
}


#[derive(Debug, Clone, PartialEq)]
pub enum Flag {
    MosquittoConf,
    MtlsConf,
    MosquittoServiceInactive,
    Null,
}
