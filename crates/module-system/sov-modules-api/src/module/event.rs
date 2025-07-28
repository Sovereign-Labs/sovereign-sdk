use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::stf::TxReceiptContents;

/// A trait that enables event processing for storage
pub trait RuntimeEventProcessor {
    /// Type specifying the wrapped enum for all events in the runtime
    type RuntimeEvent: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + serde::Serialize
        + serde::de::DeserializeOwned
        + core::fmt::Debug
        + Clone
        + PartialEq
        + Send
        + Sync
        + EventModuleName
        + Unpin;

    /// Function that converts module specific events to a wrapped event for storage
    fn convert_to_runtime_event(event: crate::TypeErasedEvent) -> Option<Self::RuntimeEvent>;
}

/// Trait to get the module name from a specific runtime event.
pub trait EventModuleName {
    /// Returns the name of the module that emitted this event.
    fn module_name(&self) -> &'static str;
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
)]
#[serde(tag = "type", rename = "moduleRef")]
/// A reference to a module
pub struct ModuleRef {
    /// The name of the module
    pub name: String,
}

/// The response type for a module specific event
#[derive(
    Debug,
    PartialEq,
    Clone,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(tag = "type", rename = "event")]
pub struct RuntimeEventResponse<E> {
    /// A global identifier for the event. Event numbers are handed out in sequential order.
    pub number: u64,
    /// Event key that was emitted along with this event
    pub key: String,
    /// A value representing the module event
    pub value: E,
    /// Module name
    pub module: ModuleRef,
    /// The hash of the transaction that emitted this event, in hex format
    pub tx_hash: HexHash,
}

impl<E> TryFrom<(u64, &sov_rollup_interface::stf::StoredEvent)> for RuntimeEventResponse<E>
where
    E: EventModuleName
        + Clone
        + borsh::BorshDeserialize
        + borsh::BorshSerialize
        + serde::Serialize
        + serde::de::DeserializeOwned,
{
    type Error = anyhow::Error;

    fn try_from(
        (event_number, stored_event): (u64, &sov_rollup_interface::stf::StoredEvent),
    ) -> Result<Self, Self::Error> {
        let runtime_event: E =
            borsh::de::BorshDeserialize::try_from_slice(stored_event.value().inner().as_slice())
                .map_err(anyhow::Error::from)?;

        let key_str = String::from_utf8(stored_event.key().inner().clone())
            .unwrap_or_else(|_| hex::encode(stored_event.key().inner()));

        let module_name = runtime_event.module_name().to_string();

        Ok(Self {
            number: event_number,
            key: key_str,
            value: runtime_event,
            module: ModuleRef { name: module_name },
            tx_hash: HexHash::from(*stored_event.tx_hash()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
/// A TxEffect as serialized for the API
#[allow(missing_docs)]
pub enum ApiTxEffect<T: TxReceiptContents> {
    Skipped { data: T::Skipped },
    Reverted { data: T::Reverted },
    Successful { data: T::Successful },
}

impl<T: TxReceiptContents> From<sov_rollup_interface::stf::TxEffect<T>> for ApiTxEffect<T> {
    fn from(value: sov_rollup_interface::stf::TxEffect<T>) -> Self {
        match value {
            sov_rollup_interface::stf::TxEffect::Skipped(data) => ApiTxEffect::Skipped { data },
            sov_rollup_interface::stf::TxEffect::Reverted(data) => ApiTxEffect::Reverted { data },
            sov_rollup_interface::stf::TxEffect::Successful(data) => {
                ApiTxEffect::Successful { data }
            }
        }
    }
}
