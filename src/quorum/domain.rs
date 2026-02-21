use serde::Deserialize;


/// Post-Failure Control and Balancing Protocol
#[derive(Default, Debug, Deserialize)]
pub struct ProtocolSettings {
    max_attempts: u64,
    time_between_heartbeats_balance_mode: u64,
    time_between_heartbeats_normal: u64,
    time_between_heartbeats_safe_mode: u64,
}


impl ProtocolSettings {
    
    pub fn get_max_attempts(&self) -> u64 {
        self.max_attempts
    }
    pub fn get_time_between_heartbeats_balance_mode(&self) -> u64 {
        self.time_between_heartbeats_balance_mode
    }
    pub fn get_time_between_heartbeats_normal(&self) -> u64 {
        self.time_between_heartbeats_normal
    }
    pub fn get_time_between_heartbeats_safe_mode(&self) -> u64 {
        self.time_between_heartbeats_safe_mode
    }
}
