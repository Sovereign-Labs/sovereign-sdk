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
