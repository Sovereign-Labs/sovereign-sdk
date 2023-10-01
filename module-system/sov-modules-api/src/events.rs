use std::marker::PhantomData;

struct WorkingSet {}
impl WorkingSet {}

// === In sov-modules-api
struct Events<EventKey, Event> {
    _phantom_key: PhantomData<EventKey>,
    _phantom_event: PhantomData<Event>,
    // Auto generated
    prefix: Vec<u8>,
}

impl<EventKey, Event> Events<EventKey, Event> {
    fn add_event(&self, event_key: EventKey, event_value: Event, ws: &mut WorkingSet) {}
}

// === In sov-foo

trait Module {
    type EventKey;
    type Event;

    fn call(&self, ws: &mut WorkingSet) -> anyhow::Result<()>;
}

type FooEventKey = &'static str;
enum FooEvent {
    E1,
    E2,
}

struct Foo {
    events: Events<FooEventKey, FooEvent>,
}

impl Module for Foo {
    type EventKey = FooEventKey;
    type Event = FooEvent;

    fn call(&self, ws: &mut WorkingSet) -> anyhow::Result<()> {
        self.events.add_event("Event_1", FooEvent::E1, ws);

        Ok(())
    }
}

// === in Runtime
struct Runtime {
    foo: Foo,
}

enum RuntimeEvent {
    foo { event_key: Vec<u8>, event: Vec<u8> },
}

impl Runtime {
    fn save_events(&self, ws: WorkingSet) {}

    fn get_events(&self, block_hash: [u8; 32], ws: WorkingSet) {}
}
