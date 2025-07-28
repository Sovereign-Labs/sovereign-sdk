use std::sync::{Arc, Mutex};

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;

/// Collects logs from the rollup.
#[derive(Clone)]
pub struct LogCollector {
    records: Arc<Mutex<Vec<(Level, String)>>>,
    level: Level,
}

impl LogCollector {
    /// Creates a new [`LogCollector`].
    pub fn new(level: Level) -> Self {
        Self {
            level,
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Recorded logs.
    pub fn records(&self) -> Vec<(Level, String)> {
        self.records.lock().unwrap().clone()
    }
}

impl<S> Layer<S> for LogCollector
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let level = *event.metadata().level();

        if level <= self.level {
            let mut message = String::new();
            let mut visitor = MessageVisitor(&mut message);
            event.record(&mut visitor);
            self.records.lock().unwrap().push((level, message));
        }
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.push_str(&format!("{value:?}"));
        }
    }
}
