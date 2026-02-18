use std::fs;
use tokio::sync::mpsc;
use rumqttc::{MqttOptions, AsyncClient, QoS, Event, Transport, Incoming, TlsConfiguration, EventLoop};
use std::time::Duration;
use tracing::error;
use crate::context::domain::AppContext;
use crate::message::domain::SerializedMessage;
use crate::network::domain::NetworkManager;
use crate::mqtt::domain::{PayloadTopic, StateClient};
use crate::system::domain::{ErrorType, InternalEvent, System};


fn create_local_mqtt(system: &System) -> Result<(AsyncClient, EventLoop), ErrorType> {

    let mut opts = MqttOptions::new(
        system.id_edge.clone(),
        system.host_local.clone(),
        8883
    );

    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_clean_session(false);

    let ca = fs::read("/etc/mosquitto/certs_edge_local/ca_edge_local.crt")?;
    let cert = fs::read("/etc/mosquitto/certs_edge_local/edge_local.crt")?;
    let key = fs::read("/etc/mosquitto/certs_edge_local/edge_local.key")?;

    opts.set_transport(Transport::Tls(
        TlsConfiguration::Simple {
            ca,
            alpn: None,
            client_auth: Some((cert, key)),
        }
    ));

    Ok(AsyncClient::new(opts, 100))
}


pub async fn mqtt(tx: mpsc::Sender<InternalEvent>,
                        mut rx_msg: mpsc::Receiver<SerializedMessage>,
                        app_context: AppContext) {
    
    let mut state = StateClient::Init;
    let mut client: Option<AsyncClient> = None;
    let mut eventloop: Option<EventLoop> = None;

    loop {
        match state {
            StateClient::Init => {
                match create_local_mqtt(&app_context.system) {
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
                                        let _ = tx.send(InternalEvent::IncomingMessage(msg));
                                    },
                                    Event::Incoming(Incoming::ConnAck(_)) => {
                                        let _ = tx.send(InternalEvent::LocalConnected);
                                    },
                                    Event::Incoming(Incoming::Disconnect) => {
                                        let _ = tx.send(InternalEvent::LocalDisconnected);
                                    },
                                    _ => {}, // KeepAlive, Pings, etc.
                                },
                                Err(e) => {
                                    error!("{}", e);
                                    let _ = tx.send(InternalEvent::LocalDisconnected);
                                    state = StateClient::Error;  // Vamos a error para reconectar
                                }
                            }
                        },

                        msg = rx_msg.recv() => {   // Enviar mensajes
                            match msg {
                                Some(msg) => {
                                    if let Some(c) = client.as_ref() {
                                        let res = c.publish(
                                            msg.get_topic().to_string(),
                                            cast_qos(&msg.get_qos()),
                                            msg.get_retain(),
                                            msg.get_payload()
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
        subs.push((net.topic_data.topic.clone(), cast_qos(&net.topic_data.qos)));
        subs.push((net.topic_monitor.topic.clone(), cast_qos(&net.topic_monitor.qos)));
        subs.push((net.topic_alert_air.topic.clone(), cast_qos(&net.topic_alert_air.qos)));
        subs.push((net.topic_alert_temp.topic.clone(), cast_qos(&net.topic_alert_temp.qos)));
        subs.push((net.topic_hub_firmware_ok.topic.clone(), cast_qos(&net.topic_hub_firmware_ok.qos)));
        subs.push((net.topic_balance_mode_handshake.topic.clone(), cast_qos(&net.topic_balance_mode_handshake.qos)));
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