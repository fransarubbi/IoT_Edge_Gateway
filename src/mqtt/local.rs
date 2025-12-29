use std::fs;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::broadcast::{Receiver as BroadcastReceiver};
use rumqttc::{MqttOptions, AsyncClient, QoS, Event, Packet, Transport, Incoming, TlsConfiguration};
use std::time::Duration;
use crate::system::fsm::{EventSystem, InternalEvent};



pub fn create_local_mqtt() -> (AsyncClient, rumqttc::EventLoop) {
    let mut opts = MqttOptions::new(
        "iot_edge_0",
        "localhost",
        8883
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



pub async fn run_local_mqtt(event_tx: Sender<InternalEvent>, mut rx_system: BroadcastReceiver<EventSystem>) {

    let enable = rx_system.recv().await.unwrap();

    match enable {
        EventSystem::EventSystemOk => {
            let (client, mut eventloop) = create_local_mqtt();
            subscribe_topics(&client).await;

            loop {
                match eventloop.poll().await {
                    Ok(event) => match event {
                        Event::Incoming(Incoming::Publish(packet)) => {
                            event_tx.send(
                                InternalEvent::FromLocalBroker(packet.payload.to_vec())
                            ).await.unwrap();
                        },
                        Event::Incoming(Incoming::ConnAck(_)) => {
                            event_tx.send(InternalEvent::LocalBrokerConnected).await.unwrap();
                        },
                        _ => {},
                    }
                    Err(_) => {
                        event_tx.send(InternalEvent::LocalBrokerDisconnected).await.unwrap();
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        },
        _ => {},
    }
}


async fn subscribe_topics(client: &AsyncClient) {
    client.subscribe("test/topic", QoS::AtMostOnce).await.unwrap();
    client.subscribe("test/monitor", QoS::AtMostOnce).await.unwrap();
    client.subscribe("test/alert", QoS::AtLeastOnce).await.unwrap();
    client.subscribe("test/settings", QoS::AtMostOnce).await.unwrap();
    client.subscribe("test/receive/network", QoS::ExactlyOnce).await.unwrap();
}