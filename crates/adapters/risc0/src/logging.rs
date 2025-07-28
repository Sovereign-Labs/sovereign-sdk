//! Helper for getting `tracing` output from inside Risc0 ZKVM
use std::fmt::Write;

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, Registry};

/// Log Layer that can be run inside zkvm guest.
/// Should be used only for debugging.
pub struct Risc0LogLayer;

impl<S> Layer<S> for Risc0LogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = meta.level();
        let target = meta.target();

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let message = match visitor.0.is_empty() {
            true => format!("{} [{}]: (no fields)", level, target),
            false => format!("{} [{}]: {}", level, target, visitor.0),
        };

        risc0_zkvm::guest::env::log(&message);
    }
}

/// A simple `Visit` implementation to collect field data from tracing events.
#[derive(Default)]
struct FieldVisitor(String);

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn core::fmt::Debug) {
        let _ = write!(&mut self.0, "{}={:?} ", field.name(), value);
    }
}

/// Initialize
pub fn init_logging(provided_filter: Option<tracing_subscriber::filter::Targets>) {
    let filter = provided_filter.unwrap_or(
        tracing_subscriber::filter::Targets::new()
            .with_default(Level::DEBUG)
            .with_target("jmt", Level::WARN),
    );
    Registry::default()
        .with(Risc0LogLayer.with_filter(filter))
        .init();
}
