use std::any::TypeId;
use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::stf::FullyBakedTx;
use sov_rollup_interface::Bytes;

use crate::{Context, HDTimestamp, Runtime, Spec, TxState};

/// A serialized, key-addressable format for sequencer-provided transaction metadata.
///
/// Implementations define how to read and prune their own serialized representation. The SDK
/// controls access tracking and only asks the format to retain the recorded keys.
///
/// # Determinism and idempotency requirements
///
/// The preferred sequencer publishes the *pruned* bytes to the DA layer and later re-executes
/// them through the same execution-and-pruning path (e.g. when replaying soft-confirmed batches
/// after a restart). For this to be sound, implementations must guarantee that:
///
/// * [`Self::get`] is deterministic: the same `bytes` and `key` always produce the same result.
/// * Reads do not depend on data that pruning may have removed. A transaction re-executed
///   against its pruned bytes must record the same set of keys as the original execution.
/// * Pruning re-serializes canonically, so that pruning already-pruned bytes with the same key
///   set is byte-for-byte identical to the input.
///
/// Violating these requirements makes the sequencer's replayed transactions diverge from the
/// bytes it published to the DA layer.
pub trait SequencingDataFormat: Send + Sync + 'static {
    /// Key type used to address entries inside the serialized sequencing data.
    type Key: BorshDeserialize + BorshSerialize + Clone + Ord + Send + Sync + 'static;

    /// Value returned for an accessed key.
    type Value;

    /// Reads the value at `key` from the serialized sequencing data.
    fn get(bytes: &[u8], key: &Self::Key) -> anyhow::Result<Option<Self::Value>>;

    /// Prunes serialized sequencing data to the keys accessed during native execution.
    ///
    /// Returning `None` removes the sequencing data from the published transaction entirely.
    /// The output must never be larger than the input and must satisfy the idempotency
    /// requirements described on [the trait](Self).
    #[cfg(feature = "native")]
    fn prune_to_used_keys(
        bytes: Bytes,
        used_keys: &std::collections::BTreeSet<Self::Key>,
    ) -> anyhow::Result<Option<Bytes>>;
}

/// Runtime support for sequencer-provided transaction metadata.
pub trait HasSequencingData<S: Spec> {
    /// Serialized sequencing data format used by this runtime.
    type SequencingData: SequencingDataFormat;

    /// Handles sequencing data before transaction dispatch.
    ///
    /// This hook is only invoked when the transaction carries sequencing data. Because unread
    /// data is pruned from the published transaction, full nodes and provers replaying the
    /// transaction may not invoke this hook (or may observe fewer keys) even though the
    /// sequencer's own execution did. To keep execution deterministic between the sequencer
    /// and replaying nodes, implementations must only let values obtained through recorded
    /// [`SequencingDataView::get`] reads influence state, and must not condition state changes
    /// on the mere presence or absence of sequencing data. Errors returned from this hook
    /// revert the transaction.
    fn handle_sequencing_data(
        &mut self,
        _data: &SequencingDataView<'_, Self::SequencingData>,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Creates serialized sequencing data for a transaction accepted by the preferred sequencer.
    #[cfg(feature = "native")]
    fn create_sequencing_data(&self) -> Option<Bytes>;
}

/// A typed view over serialized sequencing data.
///
/// In native execution, reads automatically record the accessed key in the transaction scratchpad.
/// In guest execution, the same read API is available but access recording is a no-op.
#[derive(Debug)]
pub struct SequencingDataView<'a, F: SequencingDataFormat> {
    data: Option<&'a Bytes>,
    #[cfg(feature = "native")]
    scratchpad: crate::SequencingScratchpad,
    _phantom: PhantomData<F>,
}

impl<'a, F: SequencingDataFormat> SequencingDataView<'a, F> {
    /// Creates a read-only sequencing data view.
    ///
    /// This constructor only exists in guest execution: every native view must record accessed
    /// keys (see [`Self::with_scratchpad`]), since pruning removes any key that was not
    /// recorded.
    #[cfg(not(feature = "native"))]
    pub fn new(data: Option<&'a Bytes>) -> Self {
        Self {
            data,
            _phantom: PhantomData,
        }
    }

    /// Creates a native sequencing data view that records accessed keys.
    #[cfg(feature = "native")]
    pub fn with_scratchpad(
        data: Option<&'a Bytes>,
        scratchpad: crate::SequencingScratchpad,
    ) -> Self {
        Self {
            data,
            scratchpad,
            _phantom: PhantomData,
        }
    }

    /// Reads the value at `key`, recording the access in native execution.
    pub fn get(&self, key: &F::Key) -> anyhow::Result<Option<F::Value>> {
        #[cfg(feature = "native")]
        record_used_key::<F>(&self.scratchpad, key)?;

        let Some(data) = self.data else {
            return Ok(None);
        };

        F::get(data, key)
    }
}

#[cfg(feature = "native")]
fn record_used_key<F: SequencingDataFormat>(
    scratchpad: &crate::SequencingScratchpad,
    key: &F::Key,
) -> anyhow::Result<()> {
    scratchpad.insert(borsh::to_vec(key)?);
    Ok(())
}

/// Decodes used sequencing data keys from a native scratchpad.
#[cfg(feature = "native")]
pub fn decode_used_sequencing_keys<F: SequencingDataFormat>(
    scratchpad: Option<crate::SequencingScratchpadContents>,
) -> anyhow::Result<std::collections::BTreeSet<F::Key>> {
    let Some(scratchpad) = scratchpad else {
        return Ok(std::collections::BTreeSet::new());
    };

    scratchpad
        .into_iter()
        .map(|key| F::Key::try_from_slice(&key).map_err(Into::into))
        .collect()
}

/// Applies SDK-controlled pruning to serialized sequencing data.
#[cfg(feature = "native")]
pub fn prune_sequencing_data<F: SequencingDataFormat>(
    bytes: Bytes,
    scratchpad: Option<crate::SequencingScratchpadContents>,
) -> anyhow::Result<Option<Bytes>> {
    let original_len = bytes.len();
    let used_keys = decode_used_sequencing_keys::<F>(scratchpad)?;
    let pruned = F::prune_to_used_keys(bytes, &used_keys)?;

    if let Some(pruned) = pruned.as_ref() {
        anyhow::ensure!(
            pruned.len() <= original_len,
            "pruned sequencing data grew from {} to {} bytes",
            original_len,
            pruned.len()
        );
    }

    Ok(pruned)
}

/// Extracts an [`HDTimestamp`] from the sequencing data if the runtime uses [`HDTimestamp`] as its
/// exact sequencing data format.
///
/// In some cases (i.e. inside the sequencer), we expect that the sequencing data will be valid and want to report an error if it is not.
/// In other cases, the tx may have come from an untrusted sequencer and we only want to log at the debug level.
pub fn get_maybe_timestamp_from_sequencing_data<S: Spec, Rt: Runtime<S>>(
    baked_tx: &FullyBakedTx,
    log_deserialization_error: bool,
) -> Option<HDTimestamp> {
    baked_tx.sequencing_data.as_ref().and_then(|data| {
        if TypeId::of::<<Rt as HasSequencingData<S>>::SequencingData>()
            != TypeId::of::<HDTimestamp>()
        {
            return None;
        }

        match HDTimestamp::try_from_slice(data) {
            Ok(timestamp) => Some(timestamp),
            Err(error) => {
                if log_deserialization_error {
                    tracing::error!(%error, "Failed to deserialize sequencing data");
                } else {
                    tracing::debug!(%error, "Failed to deserialize sequencing data");
                }
                None
            }
        }
    })
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use std::str::FromStr;

    use super::*;

    struct GrowingSequencingData;

    impl SequencingDataFormat for GrowingSequencingData {
        type Key = ();
        type Value = ();

        fn get(_bytes: &[u8], _key: &Self::Key) -> anyhow::Result<Option<Self::Value>> {
            Ok(Some(()))
        }

        fn prune_to_used_keys(
            _bytes: Bytes,
            _used_keys: &std::collections::BTreeSet<Self::Key>,
        ) -> anyhow::Result<Option<Bytes>> {
            Ok(Some(Bytes::from(vec![1, 2])))
        }
    }

    #[test]
    fn pruning_rejects_growth() {
        let err = prune_sequencing_data::<GrowingSequencingData>(Bytes::from(vec![1]), None)
            .expect_err("pruning must reject larger finalized sequencing data");
        assert!(err.to_string().contains("grew from 1 to 2 bytes"));
    }

    #[test]
    fn view_records_accessed_key() {
        let scratchpad = crate::SequencingScratchpad::default();
        let bytes = borsh::to_vec(&crate::HDTimestamp::from_str("42").unwrap())
            .unwrap()
            .into();
        let view = SequencingDataView::<crate::HDTimestamp>::with_scratchpad(
            Some(&bytes),
            scratchpad.clone(),
        );

        view.get(&()).unwrap();

        let used_keys = decode_used_sequencing_keys::<crate::HDTimestamp>(scratchpad.take())
            .expect("scratchpad should decode");
        assert!(used_keys.contains(&()));
    }
}
