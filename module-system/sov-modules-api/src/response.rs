use sov_rollup_interface::stf::Event;

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
