use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::stf::FullyBakedTx;

use crate::{capabilities::HasCapabilities, Context, HDTimestamp, Runtime, Spec, TxState};

/// Capability for handling sequencer-provided metadata attached to transactions.
pub trait SequencingDataHandler<S: Spec> {
    /// The decoded sequencing metadata type.
    type SequencingData: SequencingDataTrait;

    /// Handle decoded sequencing metadata.
    fn handle_sequencing_data(
        &mut self,
        _data: Self::SequencingData,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Generates the sequencing data for a new transaction.
    #[cfg(feature = "native")]
    fn create_sequencing_data(&self) -> Self::SequencingData;

    /// Finalizes sequencing data after native transaction execution.
    ///
    /// WARNING: this allows the sequencer to modify concensus data. If used carelessly, this can
    /// easily result in a broken rollup. Implementations must take great care that their usecase
    /// is consensus-safe.
    ///
    /// There are two main pitfalls to avoid:
    /// 1. Causing a modified execution outcome. If the final `sequencing_data` is mutated in such
    ///    a way that the execution output differs in *any* way (including subtle effects such as
    ///    gas usage), this result cause a faulty sequencer.
    /// 2. Growing the data size. The initial data size is used for batch size limit checks. If
    ///    finalization here grows the data, this could result in an invalid batch, soft-locking
    ///    and penalizing the preferred sequencer.
    ///
    /// A safe use for this would be if access logic uses the `scratchpad` to record
    /// used entries in the SequencingData, and after execution the sequencer prunes any unused
    /// entries to trim the transaction size. Since any entries accessed during execution remain
    /// unchanged this would be guaranteed to not change execution outcomes, and shrinking the data
    /// size is safe.
    #[cfg(feature = "native")]
    fn finalize_sequencing_data(
        &mut self,
        data: Self::SequencingData,
        _scratchpad: Option<sov_rollup_interface::Bytes>,
    ) -> Self::SequencingData {
        data
    }
}

/// Trait for sequencing data that can be deserialized and serialized using borsh.
pub trait SequencingDataTrait: BorshDeserialize + BorshSerialize + Send + Sync + 'static {
    /// Extract the timestamp from the sequencing data if it exists.
    fn get_maybe_timestamp(self) -> Option<HDTimestamp>;
}

/// Extracts the timestamp from the sequencing data if it exists.
///
/// In some cases (i.e. inside the sequencer), we expect that the sequencing data will be valid and want to report an error if it is not.
/// In other cases, the tx may have come from an untrusted sequencer and we only want to log at the debug level.
pub fn get_maybe_timestamp_from_sequencing_data<S: Spec, Rt: Runtime<S>>(
    baked_tx: &FullyBakedTx,
    log_deserialization_error: bool,
) -> Option<HDTimestamp> {
    baked_tx.sequencing_data.as_ref().and_then(|data| {
        let Ok(data) = <Rt as HasCapabilities<S>>::SequencingData::try_from_slice(data) else {
            if log_deserialization_error {
                tracing::error!("Failed to deserialize sequencing data");
            } else {
                tracing::debug!("Failed to deserialize sequencing data");
            }
            return None;
        };
        data.get_maybe_timestamp()
    })
}
