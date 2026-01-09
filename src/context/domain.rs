use std::sync::Arc;
use tokio::sync::RwLock;
use crate::database::repository::Repository;
use crate::network::domain::NetworkManager;
use crate::system::domain::System;


#[derive(Clone)]
pub struct AppContext {
    pub repo: Repository,
    pub net_man: Arc<RwLock<NetworkManager>>,  // NetworkManager es mutable y compartido, necesita Arc + RwLock. RwLock permite múltiples lecturas simultáneas, pero solo una escritura.
    pub system: Arc<System>,   // configuración de solo lectura, basta con Arc.
}


impl AppContext {
    pub fn new(repo: Repository, net_man: Arc<RwLock<NetworkManager>>, system: Arc<System>) -> Self {
        Self {
            repo,
            net_man,
            system,
        }
    }    
}