use std::fs;
use rumqttc::{MqttOptions, AsyncClient, QoS, Event, Transport, Incoming, TlsConfiguration};
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::broadcast::{Receiver as BroadcastReceiver};
use crate::system::fsm::{EventSystem, InternalEvent};




pub fn create_remote_mqtt() -> (AsyncClient, rumqttc::EventLoop) {
    let mut opts = MqttOptions::new(
        "edge-rpi-01",
        "mqtt.server.com",
        8883,
    );

    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_clean_session(false);

    let ca = fs::read("/etc/edge/certs/ca_server.crt").unwrap();
    let cert = fs::read("/etc/edge/certs/edge.crt").unwrap();
    let key = fs::read("/etc/edge/certs/edge.key").unwrap();

    opts.set_transport(Transport::Tls(
        TlsConfiguration::Simple {
            ca,
            alpn: None,
            client_auth: Some((cert, key)),
        }
    ));

    AsyncClient::new(opts, 100)
}



pub async fn run_remote_mqtt(event_tx: Sender<InternalEvent>, mut rx_system: BroadcastReceiver<EventSystem>) {
    
    let (client, mut eventloop) = create_remote_mqtt();
    subscribe_topics(&client).await;

    loop {
        match eventloop.poll().await {
            Ok(event) => match event {
                Event::Incoming(Incoming::ConnAck(_)) => {
                    event_tx.send(InternalEvent::ServerConnected).await.unwrap();
                }
                Event::Incoming(Incoming::Publish(packet)) => {
                    event_tx.send(
                        InternalEvent::FromServer(packet.payload.to_vec())
                    ).await.unwrap();
                }
                _ => {}
            },
            Err(_) => {
                event_tx.send(InternalEvent::ServerDisconnected).await.unwrap();
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}


/* Para enviar un mensaje:
    client
        .publish(
            "nodes/node_123/command",
            QoS::AtLeastOnce,
            false,
            payload_bytes,
        )
        .await?;*/


async fn subscribe_topics(client: &AsyncClient) {
    client.subscribe("server/edge/commands", QoS::AtLeastOnce).await.unwrap();
}