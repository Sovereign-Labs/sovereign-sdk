use borsh::BorshDeserialize;

use crate::{Context, Spec, TxState};

/// Capability for handling sequencer-provided metadata attached to transactions.
pub trait SequencingDataHandler<S: Spec> {
    /// The decoded sequencing metadata type.
    type SequencingData: BorshDeserialize;

    /// Decode sequencing metadata from raw bytes.
    fn decode_sequencing_data(&self, bytes: &[u8]) -> Option<Self::SequencingData> {
        Self::SequencingData::try_from_slice(bytes).ok()
    }

    /// Handle decoded sequencing metadata.
    fn handle_sequencing_data(
        &mut self,
        _data: Self::SequencingData,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
