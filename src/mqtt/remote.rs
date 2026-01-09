use std::fs;
use rumqttc::{MqttOptions, AsyncClient, QoS, Event, Transport, Incoming, TlsConfiguration, EventLoop};
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::broadcast::{Receiver as BroadcastReceiver};
use tracing::error;
use crate::context::domain::AppContext;
use crate::fsm::domain::{EventSystem, InternalEvent};
use crate::message::domain::SerializedMessage;
use crate::mqtt::domain::{PayloadTopic, StateClient};
use crate::network::domain::NetworkManager;
use crate::system::domain::{ErrorType, System};




pub fn create_server_mqtt(system: &System) -> Result<(AsyncClient, EventLoop), ErrorType> {
    let mut opts = MqttOptions::new(
        system.id_edge.clone(),
        system.host_server.clone(),
        8883,
    );

    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_clean_session(false);

    let ca = fs::read("/etc/mosquitto/certs_edge_remote/ca_edge_remote.crt")?;
    let cert = fs::read("/etc/mosquitto/certs_edge_remote/edge_remote.crt")?;
    let key = fs::read("/etc/mosquitto/certs_edge_remote/edge_remote.key")?;

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
                             mut rx_msg: Receiver<SerializedMessage>,
                             app_context: AppContext) {

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
                match create_server_mqtt(&app_context.system) {
                    Ok((c, el)) => {
                        client = Some(c.clone());
                        eventloop = Some(el);

                        let subs = {
                            let manager = app_context.net_man.read().await;
                            collect_subscriptions(&manager)
                        };

                        for (topic, qos) in subs {
                            c.subscribe(topic, qos).await.unwrap();
                        }
                        
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
                        
                        msg = rx_msg.recv() => {  
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
                                            error!("Error publicando mensaje: {:?}", e);
                                    }
                                    } else {
                                        error!("Error: Intentando publicar pero el cliente MQTT no está listo.");
                                    }
                                },
                                None => {},  // El canal se cerró
                            }
                        },
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


fn collect_subscriptions(manager: &NetworkManager) -> Vec<(String, QoS)> {
    let mut subs = Vec::new();

    for net in manager.networks.values() {
        subs.push((net.topic_network.topic.clone(), cast_qos(&net.topic_network.qos)));
        subs.push((net.topic_new_setting_to_edge.topic.clone(), cast_qos(&net.topic_new_setting_to_edge.qos)));
        subs.push((net.topic_new_setting_to_hub.topic.clone(), cast_qos(&net.topic_new_setting_to_hub.qos)));
        subs.push((net.topic_new_firmware.topic.clone(), cast_qos(&net.topic_new_firmware.qos)));
    }
    
    subs
}


fn cast_qos(qos: &u8) -> QoS {
    match qos {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}