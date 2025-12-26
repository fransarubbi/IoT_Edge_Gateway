use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};




#[derive(Debug, Clone)]
pub enum SubStateInit {
    Check,
    FirstConfig,
    Setup,
}


#[derive(Debug, Clone)]
pub enum State {
    Init(SubStateInit),
    BalanceMode,
    Normal,
    SafeMode,
    StoreMessages,
    Error,
}


#[derive(Debug)]
pub enum Event {
    CheckOk,
    CheckError,
    FirstConfigOk,
    SetupOk,
    ConfiguracionCompletada,
    CriticalError,
    Ping,
}



pub async fn run_fsm(mut rx: mpsc::Receiver<Event>) {
    let mut state = State::Init(SubStateInit::Check);

    while let Some(event) = rx.recv().await {

        match (&state, event) {

            (State::Init(SubStateInit::Check), _) => {
                println!("Apenas se pone a ejecutar la maquina, no hay eventos activo, entrara aca");


            }

            (State::Init(SubStateInit::Setup), Event::SetupOk) => {
                println!("El setup fue correcto.");
                state = State::BalanceMode;
            },

            (State::Init(SubStateInit::FirstConfig), Event::FirstConfigOk) => {
                println!("El sistema fue configurado correctamente.");
                state = State::Init(SubStateInit::Setup)
            },

            (_, Event::CriticalError) => {
                println!("¡Error detectado! Reiniciando subsistemas...");
                state = State::Error;
            },

            (State::Error, _) => {
                println!("Hacer un hard-reset, error critico!");
            },

            _ => {
                println!("Unhandled event");
            },
        }
    }
}





// --- TAREA DE CONFIGURACIÓN ---
pub async fn first_config_task(tx: mpsc::Sender<Event>) {
    println!("Verificando certificados y carpetas...");

    // En la vida real usarías tokio::fs para chequear archivos de forma asíncrona
    sleep(Duration::from_secs(2)).await;

    // Si todo sale bien o terminamos de configurar:
    println!("Configuración exitosa.");
    let _ = tx.send(Event::FirstConfigOk).await;
}



// --- TAREA DE RED / MQTT SIMULADA ---
pub async fn tarea_mqtt(tx: mpsc::Sender<Event>) {
    // Simulamos que llega un mensaje cada 3 segundos
    loop {
        sleep(Duration::from_secs(3)).await;
        //let _ = tx.send(Event::MqttMensaje("Payload Binario".to_string())).await;
    }
}






