use std::fs;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::broadcast::{Receiver as BroadcastReceiver};
use rumqttc::{MqttOptions, AsyncClient, QoS, Event, Transport, Incoming, TlsConfiguration, EventLoop};
use std::time::Duration;
use crate::message::domain::SerializedMessage;
use crate::system::check_config::ErrorType;
use crate::system::fsm::{EventSystem, InternalEvent};


#[derive(Debug)]
enum StateClient {
    Init,
    Work,
    Error,
}



pub fn create_local_mqtt() -> Result<(AsyncClient, EventLoop), ErrorType> {
    let mut opts = MqttOptions::new(
        "iot_edge_0",
        "localhost",
        8883
    );

    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_clean_session(false);

    let ca = fs::read("/etc/edge/certs/ca_server.crt")?;
    let cert = fs::read("/etc/edge/certs/edge.crt")?;
    let key = fs::read("/etc/edge/certs/edge.key")?;

    opts.set_transport(Transport::Tls(
        TlsConfiguration::Simple {
            ca,
            alpn: None,
            client_auth: Some((cert, key)),
        }
    ));

    Ok(AsyncClient::new(opts, 100))
}



pub async fn run_local_mqtt(event_tx: Sender<InternalEvent>, mut rx_system: BroadcastReceiver<EventSystem>, mut rx_msg: Receiver<SerializedMessage>) {

    loop {
        match rx_system.recv().await {    // .recv().await suspende la tarea, no gasta CPU
            Ok(EventSystem::EventSystemOk) => break,
            Err(_) => return,
            _ => {},
        }
    }

    let mut state = StateClient::Init;
    let mut client: Option<AsyncClient> = None;
    let mut eventloop: Option<EventLoop> = None;

    loop {
        match state {
            StateClient::Init => {
                match create_local_mqtt() {
                    Ok((c, el)) => {
                        client = Some(c.clone());
                        eventloop = Some(el);
                        subscribe_topics(&c).await;
                        state = StateClient::Work;
                    },
                    Err(e) => {
                        state = StateClient::Error;
                    }
                }
            },
            StateClient::Work => {
                if let Some(el) = eventloop.as_mut() {
                    tokio::select! {  // select! permite recibir comandos del sistema mientras se escucha MQTT
                        notification = el.poll() => {
                            match notification {
                                Ok(event) => match event {
                                    Event::Incoming(Incoming::Publish(packet)) => {
                                        let _ = event_tx.send(
                                            InternalEvent::FromLocalBroker(packet.payload.to_vec())
                                        ).await;
                                    },
                                    Event::Incoming(Incoming::ConnAck(_)) => {
                                        let _ = event_tx.send(InternalEvent::LocalBrokerConnected).await;
                                    },
                                    _ => {}, // KeepAlive, Pings, etc.
                                },
                                Err(e) => {
                                    let _ = event_tx.send(InternalEvent::LocalBrokerDisconnected).await;
                                    state = StateClient::Error;  // Vamos a error para reconectar
                                }
                            }
                        },

                        msg = rx_msg.recv() => {   // Enviar mensajes
                            match msg {
                                Some(msg) => {
                                    if let Some(c) = client.as_ref() {
                                        let res = c.publish(
                                            msg.topic,
                                            cast_qos(msg.qos),
                                            msg.retain,
                                            msg.payload
                                        ).await;
                                        
                                    if let Err(e) = res {
                                            eprintln!("Error publicando mensaje: {:?}", e);
                                    }
                                    } else {
                                        eprintln!("Error: Intentando publicar pero el cliente MQTT no está listo.");
                                    }
                                },
                                None => {
                                    // El canal se cerró
                                },
                            }
                        },
                    }
                } else {   // Si por algún bug llegamos a Work sin eventloop, volvemos a Init
                    state = StateClient::Init;
                }
            },
            StateClient::Error => {
                tokio::time::sleep(Duration::from_secs(5)).await;  // Logica de backoff (esperar antes de reintentar)
                state = StateClient::Init;   // Intentamos reconectar volviendo a Init
            },
        }
    }
}


async fn subscribe_topics(client: &AsyncClient) {
    client.subscribe("test/topic", QoS::AtMostOnce).await.unwrap();
    client.subscribe("test/monitor", QoS::AtMostOnce).await.unwrap();
    client.subscribe("test/alert", QoS::AtLeastOnce).await.unwrap();
    client.subscribe("test/settings", QoS::AtMostOnce).await.unwrap();
    client.subscribe("test/receive/network", QoS::ExactlyOnce).await.unwrap();
}


fn cast_qos(qos: u8) -> QoS {
    match qos {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}