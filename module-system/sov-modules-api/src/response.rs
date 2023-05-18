use sovereign_core::stf::Event;

/// Response type for the `Module::call` method.
#[derive(Default)]
pub struct CallResponse {
    /// Lists of events emitted by a call to a module.
    pub events: Vec<Event>,
}

impl CallResponse {
    pub fn add_event(&mut self, key: &str, value: &str) {
        self.events.push(Event::new(key, value))
    }
}

/// Response type for the `Module::query` method. The response is returned by the relevant RPC call.
#[derive(Default, Debug)]
pub struct QueryResponse {
    pub response: Vec<u8>,
}
