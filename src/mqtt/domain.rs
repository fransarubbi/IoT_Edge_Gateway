#[derive(Debug, Clone, PartialEq)]
pub struct PayloadTopic {
    pub payload: Vec<u8>,
    pub topic: String,
}

impl PayloadTopic {
    pub fn new(payload: Vec<u8>, topic: String) -> Self {
        Self { payload, topic }
    }
}


#[derive(Debug)]
pub enum StateClient {
    Init,
    Work,
    Error,
}