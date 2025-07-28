use sov_modules_api::macros::serialize;

/// Sample Event
#[derive(Debug, PartialEq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// CPU Heavy Operation event
    RanCPUHeavyOperation(u64, Vec<u8>),
    /// Read and set many individual values event
    ReadAndSetManyIndividualValues(u64),
    /// Read and set heavy state event
    ReadAndSetHeavyState(u64, u64),
}
