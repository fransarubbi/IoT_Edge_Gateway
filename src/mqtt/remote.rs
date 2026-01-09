use std::fs;
use rumqttc::{MqttOptions, AsyncClient, QoS, Event, Transport, Incoming, TlsConfiguration, EventLoop};
use std::time::Duration;
use tokio::sync::mpsc::{Sender};
use tokio::sync::broadcast::{Receiver as BroadcastReceiver};
use crate::fsm::domain::{EventSystem, InternalEvent};
use crate::mqtt::domain::PayloadTopic;
use crate::network::domain::NetworkManager;
use crate::system::check::ErrorType;
use crate::system::domain::System;


#[derive(Debug)]
enum StateClient {
    Init,
    Work,
    Error,
}


pub fn create_server_mqtt(system: &System) -> Result<(AsyncClient, EventLoop), ErrorType> {
    let mut opts = MqttOptions::new(
        system.id_edge.clone(),
        system.host_server.clone(),
        8883,
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



pub async fn run_remote_mqtt(event_tx: Sender<InternalEvent>,
                             mut rx_system: BroadcastReceiver<EventSystem>,
                             net_man: &NetworkManager,
                             system: &System) {

    loop {
        match rx_system.recv().await {
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
                match create_server_mqtt(system) {
                    Ok((c, el)) => {
                        client = Some(c.clone());
                        eventloop = Some(el);
                        subscribe_topics(&c, net_man).await;
                        state = StateClient::Work;
                    },
                    Err(e) => {
                        log::error!("{}", e);
                        state = StateClient::Error;
                    }
                }
            },
            StateClient::Work => {
                if let Some(el) = eventloop.as_mut() {
                    tokio::select! {
                        notification = el.poll() => {
                            match notification {
                                Ok(event) => match event {
                                    Event::Incoming(Incoming::Publish(packet)) => {
                                        let msg = PayloadTopic::new(
                                            packet.payload.to_vec(),
                                            packet.topic
                                        );
                                        let _ = event_tx.send(
                                            InternalEvent::IncomingMessage(msg)
                                        ).await;
                                    },
                                    Event::Incoming(Incoming::ConnAck(_)) => {
                                        let _ = event_tx.send(InternalEvent::LocalConnected).await;
                                    },
                                    _ => {}, // KeepAlive, Pings, etc.
                                },
                                Err(e) => {
                                    log::error!("{}", e);
                                    let _ = event_tx.send(InternalEvent::LocalDisconnected).await;
                                    state = StateClient::Error;  // Vamos a error para reconectar
                                }
                            }
                        },

                        /*
                        msg = rx_msg.recv() => {   // Enviar mensajes
                            match msg {
                                Some(msg) => {
                                    if let Some(c) = client.as_ref() {
                                        let res = c.publish(
                                            msg.topic,
                                            cast_qos(&msg.qos),
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
                                None => {},  // El canal se cerró
                            }
                        },*/
                    }
                } else {   // Si por algún bug llegamos a Work sin eventloop, volvemos a Init
                    state = StateClient::Init;
                }
            },
            StateClient::Error => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                state = StateClient::Init;
            },
        }
    }
}




async fn subscribe_topics(client: &AsyncClient, net_man: &NetworkManager) {

    for network in &net_man.networks {
        client.subscribe(&network.1.topic_data.topic, cast_qos(&network.1.topic_data.qos)).await.unwrap();
        client.subscribe(&network.1.topic_monitor.topic, cast_qos(&network.1.topic_monitor.qos)).await.unwrap();
        client.subscribe(&network.1.topic_alert_air.topic, cast_qos(&network.1.topic_alert_air.qos)).await.unwrap();
        client.subscribe(&network.1.topic_alert_temp.topic, cast_qos(&network.1.topic_alert_temp.qos)).await.unwrap();
        client.subscribe(&network.1.topic_settings_from_hub.topic, cast_qos(&network.1.topic_settings_from_hub.qos)).await.unwrap();
    }
}


fn cast_qos(qos: &u8) -> QoS {
    match qos {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}