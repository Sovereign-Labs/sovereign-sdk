use sov_modules_api::macros::serialize;

/// Template Event
#[derive(Debug, PartialEq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// Template event set value
    Set { value: u32 },
}
