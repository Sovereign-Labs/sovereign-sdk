use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::stf::TxReceiptContents;

/// Identifies a run of event numbering.
///
/// Event numbers are handed out by the sequencer before the node commits them, so a rollback
/// can retract numbered events and hand the same numbers out again for different content. The
/// number alone is therefore not a stable identifier: event 95 before a rollback and event 95
/// after it are different events, and a rollback that retracts fewer events than it later
/// re-emits is invisible to anyone watching only the number.
///
/// The epoch closes that hole. It changes whenever the numbering is reissued, so `(epoch,
/// number)` *is* stable: two events agreeing on both are the same event.
///
/// It is an opaque token — compare it for equality and nothing else. It carries no ordering,
/// and a fresh one is minted whenever the sequencer rewinds *or restarts*, so it never needs
/// to be persisted. On the wire it is a fixed-width lowercase hex string.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, borsh::BorshDeserialize, borsh::BorshSerialize,
)]
pub struct EventEpoch(u128);

impl EventEpoch {
    /// Wraps an opaque token minted by whoever hands out event numbers.
    pub fn new(token: u128) -> Self {
        Self(token)
    }

    /// The raw token, for transport.
    pub fn get(self) -> u128 {
        self.0
    }
}

impl core::fmt::Display for EventEpoch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl serde::Serialize for EventEpoch {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for EventEpoch {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let hex = <String as serde::Deserialize>::deserialize(deserializer)?;
        u128::from_str_radix(&hex, 16)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

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
    /// The run of numbering [`Self::number`] belongs to, when that numbering is speculative.
    ///
    /// Present on events served from the sequencer, whose numbers are handed out before the
    /// node commits them and are reissued if the sequencer rolls back. Compare it against the
    /// epoch a state snapshot reported before applying this event to that snapshot: if they
    /// differ, the number refers to a run of events the snapshot knows nothing about.
    ///
    /// Absent on events read back from the ledger. Those are committed, so their numbering is
    /// canonical and is never handed out again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<EventEpoch>,
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
            // Read back from the ledger, so committed: the numbering is canonical and will
            // never be handed out again, which is exactly what an absent epoch means.
            epoch: None,
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
