use sov_modules_api::macros::serialize;

/// Sample Event
#[derive(Debug, PartialEq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// Sample event variant 1
    Event1,
    /// Sample event variant 2
    Event2,
}
