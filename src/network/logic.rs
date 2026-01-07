use crate::database::repository::Repository;
use crate::network::domain::NetworkManager;


pub async fn create_network_manager() -> NetworkManager {

    let net_man = NetworkManager::new(
        "edge0/message/handshake".to_string(),
        0,
        "edge0/message/state".to_string(),
        1
    );

    net_man
}


pub async fn load_networks(repo: &Repository, net_man: &mut NetworkManager) {
    match repo.get_all_network().await {
        Ok(networks) => {
            for network in networks {
                net_man.add_network(network);
            }
        },
        Err(e) => {
            log::error!("{}", e);
        },
    }
}

