pub mod message {

}

pub mod mqtt {

}

pub mod system {

}

pub mod sqlite {
    use tokio::time::{Duration};

    pub const LIMIT: i64 = 10;
    pub const WAIT_FOR: u64 = 5;
    pub const BATCH_SIZE: usize = 100;
    pub const TABLE : usize = 4;
    pub const FLUSH_INTERVAL: Duration = Duration::from_secs(5);
}

pub mod firmware {
    use tokio::time::{Duration};
    pub const OTA_TIMEOUT: Duration = Duration::from_secs(300); // 5 min
}

pub mod fsm {
    use tokio::time::{Duration};
    pub const HELLO_TIMEOUT: Duration = Duration::from_secs(180); // 3 min
}

pub mod grpc_service {
    pub const TLS_CERT_TIMEOUT_SECS: u64 = 10;
    pub const KEEP_ALIVE_TIMEOUT_SECS: u64 = 30;
    pub const RECONNECT_DELAY_SECS: u64 = 5;
    pub const SESSION_CHANNEL_BUFFER: usize = 100;
    pub const GZIP_ENABLED: bool = true;
}

pub mod heartbeat {
    use tokio::time::{Duration};
    pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(50);
}