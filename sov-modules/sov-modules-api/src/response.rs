use std::rc::Rc;

use sovereign_sdk::stf::{Event, EventKey, EventValue};

/// Response type for the `Module::call` method.
#[derive(Default)]
pub struct CallResponse {
    /// Lists of events emitted by a call to a module.
    events: Vec<Event>,
}

impl CallResponse {
    pub fn add_event(&mut self, key: &str, value: &str) {
        let event = Event {
            key: EventKey(Rc::new(key.as_bytes().to_vec())),
            value: EventValue(Rc::new(value.as_bytes().to_vec())),
        };

        self.events.push(event)
    }
}

/// Response type for the `Module::query` method. The response is returned by the relevant RPC call.
#[derive(Default, Debug)]
pub struct QueryResponse {
    pub response: Vec<u8>,
}
