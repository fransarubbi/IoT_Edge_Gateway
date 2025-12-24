
#[derive(Debug, Clone)]
pub struct Network {
    id_network: String,
    name_network: String,
    topic_data: String,
    topic_alert: String,
    topic_monitor: String,
    active: bool,
}

impl Network {
    //Getters
    pub fn id_network(&self) -> &str { &self.id_network }
    pub fn name_network(&self) -> &str { &self.name_network }
    pub fn topic_data(&self) -> &str { &self.topic_data }
    pub fn topic_alert(&self) -> &str { &self.topic_alert }
    pub fn topic_monitor(&self) -> &str { &self.topic_monitor }
    pub fn active(&self) -> &bool { &self.active }

    //Setters
    pub fn set_id_network(&mut self, id_network: String) { self.id_network = id_network; }
    pub fn set_name_network(&mut self, name_network: String) { self.name_network = name_network; }
    pub fn set_topic_data(&mut self, topic_data: String) { self.topic_data = topic_data; }
    pub fn set_topic_alert(&mut self, topic_alert: String) { self.topic_alert = topic_alert; }
    pub fn set_topic_monitor(&mut self, topic_monitor: String) { self.topic_monitor = topic_monitor; }
    pub fn set_active(&mut self, active: bool) { self.active = active; }
}

#[derive(Debug, Clone)]
pub struct NetworkBuilder {
    id_network: Option<String>,
    name_network: Option<String>,
    topic_data: Option<String>,
    topic_alert: Option<String>,
    topic_monitor: Option<String>,
    active: Option<bool>,
}

impl NetworkBuilder {
    pub fn new() -> Self {
        Self {
            id_network: None,
            name_network: None,
            topic_data: None,
            topic_alert: None,
            topic_monitor: None,
            active: None,
        }
    }
    pub fn id_network(mut self, id_network: String) -> Self {
        self.id_network = Some(id_network);
        self
    }
    pub fn name_network(mut self, name_network: String) -> Self {
        self.name_network = Some(name_network);
        self
    }
    pub fn topic_data(mut self, topic_data: String) -> Self {
        self.topic_data = Some(topic_data);
        self
    }
    pub fn topic_alert(mut self, topic_alert: String) -> Self {
        self.topic_alert = Some(topic_alert);
        self
    }
    pub fn topic_monitor(mut self, topic_monitor: String) -> Self {
        self.topic_monitor = Some(topic_monitor);
        self
    }
    pub fn active(mut self, is_active: bool) -> Self {
        self.active = Some(is_active);
        self
    }
    pub fn build(self) -> Result<Network, String> {
        Ok(Network {
            id_network: self.id_network.unwrap_or_else(|| "unknown".to_string()),
            name_network: self.name_network.unwrap_or_else(|| "unknown".to_string()),
            topic_data: self.topic_data.unwrap_or_else(|| "unknown".to_string()),
            topic_alert: self.topic_alert.unwrap_or_else(|| "unknown".to_string()),
            topic_monitor: self.topic_monitor.unwrap_or_else(|| "unknown".to_string()),
            active: self.active.unwrap_or(true),
        })
    }
}