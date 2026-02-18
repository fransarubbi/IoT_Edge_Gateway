use tokio::sync::mpsc;
use tracing::error;
use crate::context::domain::AppContext;
use crate::message::domain::SerializedMessage;
use crate::mqtt::local::mqtt;
use crate::system::domain::InternalEvent;


pub struct MqttService {
    sender: mpsc::Sender<InternalEvent>,
    receiver: mpsc::Receiver<SerializedMessage>,
    context: AppContext
}


impl MqttService {
    pub fn new(sender: mpsc::Sender<InternalEvent>,
               receiver: mpsc::Receiver<SerializedMessage>,
               context: AppContext) -> Self {
        Self {
            sender,
            receiver,
            context
        }
    }

    pub async fn run(mut self) {

        let (tx, mut rx_response) = mpsc::channel::<InternalEvent>(100);
        let (tx_command, rx_msg) = mpsc::channel::<SerializedMessage>(100);

        tokio::spawn(mqtt(tx, rx_msg, self.context.clone()));

        loop {
            tokio::select! {
                Some(msg) = self.receiver.recv() => {
                    if tx_command.send(msg).await.is_err() {
                        error!("Error: no se pudo enviar SerializedMessage desde MqttService");
                    }
                }
                Some(msg) = rx_response.recv() => {
                    if self.sender.send(msg).await.is_err() {
                        error!("Error: no se pudo enviar InternalEvent desde MqttService");
                    }
                }
            }
        }
    }
}




#[derive(Debug, Clone, PartialEq)]
pub struct PayloadTopic {
    pub payload: Vec<u8>,
    pub topic: String,
}

impl PayloadTopic {
    pub fn new(payload: Vec<u8>, topic: String) -> Self {
        Self { payload, topic }
    }
}


#[derive(Debug)]
pub enum StateClient {
    Init,
    Work,
    Error,
}

