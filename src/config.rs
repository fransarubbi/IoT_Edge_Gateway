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