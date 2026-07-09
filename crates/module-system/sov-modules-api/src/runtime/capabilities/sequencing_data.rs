use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::stf::FullyBakedTx;
use sov_rollup_interface::Bytes;

use crate::{Context, HDTimestamp, SequencingData, Spec, TxState};

/// A serialized, key-addressable format for sequencer-provided transaction metadata.
///
/// Implementations define how to read and prune their own serialized representation. The SDK
/// controls access tracking and only asks the format to retain the recorded keys.
///
/// The format only governs the `data` payload of [`SequencingData`]; the accompanying timestamp
/// is SDK-managed and never pruned.
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
    ///
    /// Serialization must be infallible: accessed keys are recorded during native execution
    /// and the SDK panics if serializing one fails. A fallible serializer cannot be allowed to
    /// surface as an error, because recording only happens in native builds — the zk guest
    /// would execute the same read successfully and diverge from the full node.
    type Key: BorshDeserialize + BorshSerialize + Clone + Ord + Send + Sync + 'static;

    /// Value returned for an accessed key.
    type Value;

    /// Reads the value at `key` from the serialized sequencing data.
    ///
    /// Absent keys must be reported as `Ok(None)`, and absence must be stable under pruning:
    /// the SDK does not record reads that return `Ok(None)`, relying on a key absent from the
    /// full payload also being absent from the pruned payload.
    fn get(bytes: &[u8], key: &Self::Key) -> anyhow::Result<Option<Self::Value>>;

    /// Prunes serialized sequencing data to the keys accessed during native execution.
    ///
    /// Returning `None` removes the data payload from the published transaction entirely.
    /// The output must never be larger than the input and must satisfy the idempotency
    /// requirements described on [the trait](Self).
    #[cfg(feature = "native")]
    fn prune_to_used_keys(
        bytes: Bytes,
        used_keys: &std::collections::BTreeSet<Self::Key>,
    ) -> anyhow::Result<Option<Bytes>>;

    /// Creates the serialized sequencing data that the preferred sequencer attaches to each
    /// accepted transaction.
    ///
    /// Returning `None` (the default) attaches no format-specific data; the SDK still attaches
    /// a timestamp alongside it (see [`new_tx_sequencing_data`]).
    ///
    /// The output must fit within [`MAX_FULLY_BAKED_TX_SIZE`](sov_rollup_interface::stf::MAX_FULLY_BAKED_TX_SIZE)
    /// together with the transaction it is attached to; a payload that alone exceeds that limit
    /// is discarded by [`new_tx_sequencing_data`], since no node could ever deserialize the
    /// resulting transaction.
    #[cfg(feature = "native")]
    fn create() -> Option<Bytes> {
        None
    }
}

/// The trivial sequencing data format for runtimes that use no format-specific sequencing data.
///
/// Transactions still carry the SDK-managed timestamp of [`SequencingData`].
impl SequencingDataFormat for () {
    type Key = ();
    type Value = std::convert::Infallible;

    fn get(_bytes: &[u8], _key: &Self::Key) -> anyhow::Result<Option<Self::Value>> {
        Ok(None)
    }

    #[cfg(feature = "native")]
    fn prune_to_used_keys(
        _bytes: Bytes,
        _used_keys: &std::collections::BTreeSet<Self::Key>,
    ) -> anyhow::Result<Option<Bytes>> {
        Ok(None)
    }
}

/// Runtime support for sequencer-provided transaction metadata.
///
/// Runtimes without custom sequencing data only need to set the associated type:
///
/// ```ignore
/// impl<S: Spec> HasSequencingData<S> for MyRuntime<S> {
///     type SequencingData = ();
/// }
/// ```
///
/// The sequencer-attached timestamp is handled by the SDK independently of this trait: the STF
/// updates the chain-state time oracle from it before dispatching each preferred-sequencer
/// transaction.
pub trait HasSequencingData<S: Spec> {
    /// Serialized sequencing data format used by this runtime.
    type SequencingData: SequencingDataFormat;

    /// Handles sequencing data before transaction dispatch.
    ///
    /// This hook is only invoked when the transaction carries sequencing data, which is only
    /// ever the case for preferred-sequencer transactions: data attached by any other sequencer
    /// is discarded before execution. Because unread
    /// data is pruned from the published transaction, full nodes and provers replaying the
    /// transaction may observe fewer keys than the sequencer's own execution did. To keep
    /// execution deterministic between the sequencer and replaying nodes, implementations must
    /// only let values obtained through recorded [`SequencingDataView::get`] reads influence
    /// state, and must not condition state changes on the mere presence or absence of
    /// sequencing data. Errors returned from this hook revert the transaction.
    fn handle_sequencing_data(
        &mut self,
        _data: &SequencingDataView<'_, Self::SequencingData>,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Builds the full serialized [`SequencingData`] (timestamp plus format-provided data) that
/// the preferred sequencer attaches to a transaction when accepting it.
///
/// A data payload that alone exceeds
/// [`MAX_FULLY_BAKED_TX_SIZE`](sov_rollup_interface::stf::MAX_FULLY_BAKED_TX_SIZE) is discarded
/// with an error log: a transaction carrying it could never be published, since every node's
/// deserializer rejects it, and encoding an unbounded payload risks a panic. Execution then
/// proceeds without a data payload, which stays deterministic across all node types.
#[cfg(feature = "native")]
pub fn new_tx_sequencing_data<S: Spec, Rt: HasSequencingData<S>>() -> Bytes {
    let data = Rt::SequencingData::create().filter(|data| {
        let fits = data.len() <= sov_rollup_interface::stf::MAX_FULLY_BAKED_TX_SIZE;
        if !fits {
            tracing::error!(
                payload_len = data.len(),
                max_len = sov_rollup_interface::stf::MAX_FULLY_BAKED_TX_SIZE,
                "SequencingDataFormat::create() returned a payload exceeding the maximum \
                 transaction size; attaching no data payload"
            );
        }
        fits
    });
    SequencingData {
        timestamp: Some(HDTimestamp::default_sequencing_data()),
        data,
    }
    .encode()
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
    ///
    /// Reads that observe nothing (`Ok(None)`) are not recorded: pruning can only remove
    /// entries, so a key absent from the full payload is also absent from the pruned payload
    /// and the replayed read observes `Ok(None)` again. Errors ARE recorded like successful
    /// reads, so that a replayed read of e.g. a corrupt entry observes the same error instead
    /// of `Ok(None)`.
    pub fn get(&self, key: &F::Key) -> anyhow::Result<Option<F::Value>> {
        // Payload-less reads skip recording: pruning ignores the scratchpad when the envelope
        // carries no data payload, so a record here could never be observed.
        let Some(data) = self.data else {
            return Ok(None);
        };

        let result = F::get(data, key);

        #[cfg(feature = "native")]
        if !matches!(result, Ok(None)) {
            // Panic rather than propagate: this write only exists in native builds, so a
            // fallible key serializer surfacing as an error would revert the transaction on
            // full nodes while the zk guest (which never records) executes it successfully —
            // a consensus divergence. See the infallibility requirement on
            // `SequencingDataFormat::Key`.
            let mut encoded_key = Vec::new();
            borsh::BorshSerialize::serialize(key, &mut encoded_key)
                .expect("sequencing data keys must serialize infallibly");
            self.scratchpad.insert(encoded_key);
        }

        result
    }
}

/// Decodes used sequencing data keys from a native scratchpad.
#[cfg(feature = "native")]
pub fn decode_used_sequencing_keys<F: SequencingDataFormat>(
    scratchpad: crate::SequencingScratchpadContents,
) -> anyhow::Result<std::collections::BTreeSet<F::Key>> {
    scratchpad
        .into_iter()
        .map(|key| {
            F::Key::try_from_slice(&key).map_err(|error| {
                anyhow::anyhow!(
                    "recorded sequencing data key {key:02x?} does not decode as the key of \
                     `{}`: {error}. Was a `SequencingDataView` instantiated with a format \
                     other than the runtime's `HasSequencingData::SequencingData`?",
                    std::any::type_name::<F>()
                )
            })
        })
        .collect()
}

/// Applies SDK-controlled pruning to serialized sequencing data.
///
/// The timestamp (if any) is always preserved; only the format-specific data payload is pruned
/// to the keys recorded during native execution. Returns `None` if neither a timestamp nor any
/// data remains.
#[cfg(feature = "native")]
pub fn prune_sequencing_data<F: SequencingDataFormat>(
    bytes: Bytes,
    scratchpad: crate::SequencingScratchpadContents,
) -> anyhow::Result<Option<Bytes>> {
    let original_len = bytes.len();
    let envelope = SequencingData::decode(&bytes)?;

    let Some(data) = envelope.data else {
        // There is no data payload to prune. The codec encodes every decodable value uniquely,
        // so re-encoding would reproduce `bytes` exactly; keep them as-is (or drop an entirely
        // empty envelope).
        return Ok(envelope.timestamp.is_some().then_some(bytes));
    };

    let used_keys = decode_used_sequencing_keys::<F>(scratchpad)?;
    let pruned = SequencingData {
        timestamp: envelope.timestamp,
        data: F::prune_to_used_keys(data, &used_keys)?,
    };
    if pruned.is_empty() {
        return Ok(None);
    }

    let pruned = pruned.encode();
    anyhow::ensure!(
        pruned.len() <= original_len,
        "pruned sequencing data grew from {} to {} bytes",
        original_len,
        pruned.len()
    );
    Ok(Some(pruned))
}

/// Extracts the sequencer-attached timestamp from a transaction's sequencing data, if any.
///
/// In some cases (i.e. inside the sequencer), we expect that the sequencing data will be valid and want to report an error if it is not.
/// In other cases, the tx may have come from an untrusted sequencer and we only want to log at the debug level.
pub fn get_timestamp_from_sequencing_data(
    baked_tx: &FullyBakedTx,
    log_deserialization_error: bool,
) -> Option<HDTimestamp> {
    let data = baked_tx.sequencing_data.as_ref()?;
    match SequencingData::decode(data) {
        Ok(sequencing_data) => sequencing_data.timestamp,
        Err(error) => {
            if log_deserialization_error {
                tracing::error!(%error, "Failed to deserialize sequencing data");
            } else {
                tracing::debug!(%error, "Failed to deserialize sequencing data");
            }
            None
        }
    }
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

    struct DroppingSequencingData;

    impl SequencingDataFormat for DroppingSequencingData {
        type Key = ();
        type Value = ();

        fn get(_bytes: &[u8], _key: &Self::Key) -> anyhow::Result<Option<Self::Value>> {
            Ok(Some(()))
        }

        fn prune_to_used_keys(
            _bytes: Bytes,
            _used_keys: &std::collections::BTreeSet<Self::Key>,
        ) -> anyhow::Result<Option<Bytes>> {
            Ok(None)
        }
    }

    fn encoded_envelope(timestamp: Option<HDTimestamp>, data: Option<Vec<u8>>) -> Bytes {
        SequencingData {
            timestamp,
            data: data.map(Into::into),
        }
        .encode()
    }

    #[test]
    fn pruning_rejects_growth() {
        let bytes = encoded_envelope(None, Some(vec![1]));
        let original_len = bytes.len();
        let err = prune_sequencing_data::<GrowingSequencingData>(bytes, Default::default())
            .expect_err("pruning must reject larger finalized sequencing data");
        assert!(err.to_string().contains(&format!(
            "grew from {original_len} to {} bytes",
            original_len + 1
        )));
    }

    #[test]
    fn pruning_preserves_timestamp_when_data_is_dropped() {
        let timestamp = HDTimestamp::from_str("17123456789012345678").unwrap();
        let bytes = encoded_envelope(Some(timestamp), Some(vec![1, 2, 3]));

        let pruned = prune_sequencing_data::<DroppingSequencingData>(bytes, Default::default())
            .expect("pruning should succeed")
            .expect("the timestamp must survive pruning");
        assert_eq!(
            SequencingData::decode(&pruned).unwrap(),
            SequencingData {
                timestamp: Some(timestamp),
                data: None
            },
            "pruning must keep the timestamp and drop only the data payload"
        );
    }

    #[test]
    fn pruning_removes_empty_envelope() {
        let bytes = encoded_envelope(None, Some(vec![1, 2, 3]));

        let pruned = prune_sequencing_data::<DroppingSequencingData>(bytes, Default::default())
            .expect("pruning should succeed");
        assert_eq!(
            pruned, None,
            "sequencing data with no timestamp and fully pruned data must be removed"
        );
    }

    struct KeyedSequencingData;

    impl SequencingDataFormat for KeyedSequencingData {
        type Key = u16;
        type Value = ();

        fn get(_bytes: &[u8], _key: &Self::Key) -> anyhow::Result<Option<Self::Value>> {
            Ok(Some(()))
        }

        fn prune_to_used_keys(
            bytes: Bytes,
            _used_keys: &std::collections::BTreeSet<Self::Key>,
        ) -> anyhow::Result<Option<Bytes>> {
            Ok(Some(bytes))
        }
    }

    #[test]
    fn view_records_accessed_key() {
        let scratchpad = crate::SequencingScratchpad::default();
        let bytes = Bytes::from(vec![0x12, 0x34]);
        let view = SequencingDataView::<KeyedSequencingData>::with_scratchpad(
            Some(&bytes),
            scratchpad.clone(),
        );

        view.get(&0x1234).unwrap();

        let used_keys = decode_used_sequencing_keys::<KeyedSequencingData>(scratchpad.take())
            .expect("scratchpad should decode");
        assert!(used_keys.contains(&0x1234));
    }

    /// Format whose `get` observes no value for any key.
    struct AbsentKeySequencingData;

    impl SequencingDataFormat for AbsentKeySequencingData {
        type Key = u16;
        type Value = ();

        fn get(_bytes: &[u8], _key: &Self::Key) -> anyhow::Result<Option<Self::Value>> {
            Ok(None)
        }

        fn prune_to_used_keys(
            bytes: Bytes,
            _used_keys: &std::collections::BTreeSet<Self::Key>,
        ) -> anyhow::Result<Option<Bytes>> {
            Ok(Some(bytes))
        }
    }

    #[test]
    fn view_does_not_record_absent_key() {
        let scratchpad = crate::SequencingScratchpad::default();
        let bytes = Bytes::from(vec![0x12, 0x34]);
        let view = SequencingDataView::<AbsentKeySequencingData>::with_scratchpad(
            Some(&bytes),
            scratchpad.clone(),
        );

        assert_eq!(view.get(&0x1234).unwrap(), None);

        assert_eq!(
            scratchpad.take(),
            Default::default(),
            "reads observing no value must not be recorded: an absent key stays absent after \
             pruning, so the record would only bloat the published data"
        );
    }
}
