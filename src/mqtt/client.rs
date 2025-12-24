use rumqttc::{MqttOptions, AsyncClient, QoS, Event, Packet};
use std::time::Duration;
use rmp_serde::from_slice;
use crate::message::msg_type::{Message, parse_message};


pub async fn run_mqtt() {
    let mut opts = MqttOptions::new("iot_edge_0", "localhost", 8883);
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_clean_session(false);
    opts.set_credentials("user", "#iot2025");

    // TODO: Certificados

    let (client, mut eventloop) = AsyncClient::new(opts, 100);

    // Nos suscribimos
    // Mejor manejar el error, no usar unwrap()
    client.subscribe("iot/config/#", QoS::AtLeastOnce).await.unwrap();
    client.subscribe("iot/sensors/#", QoS::AtLeastOnce).await.unwrap();

    // Spawn del loop de eventos
    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(notification) => {
                    match notification {
                        Event::Incoming(Packet::Publish(packet)) => {
                            // packet.payload son los bytes crudos (MessagePack)
                            println!("Mensaje recibido en: {}", packet.topic);

                            match from_slice::<Message>(&packet.payload) {
                                Ok(mut mensaje_decodificado) => {
                                    parse_message(&mut mensaje_decodificado);
                                    println!("Mensaje entendido: {:?}", mensaje_decodificado);
                                },
                                Err(e) => {
                                    eprintln!("Error decodificando MessagePack: {}", e);
                                }
                            }
                        },
                        Event::Incoming(Packet::ConnAck(_)) => {
                            println!("¡Conectado al Broker exitosamente!");
                        },
                        _ => {} // Ignorar PINGs, ACKs de publicación, etc.
                    }
                },
                Err(e) => {
                    eprintln!("Error de conexión MQTT: {:?}. Reintentando...", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });
}