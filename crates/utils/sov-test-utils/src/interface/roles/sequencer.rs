use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use sov_mock_da::MockDaSpec;
use sov_modules_api::{Amount, DaSpec, Runtime, Spec};

use super::{AsUser, TestUser};
use crate::{BatchType, SequencerInfo, SoftConfirmationBlobInfo};

/// A representation of a preferred sequencer at genesis
pub struct TestPreferredSequencer<S: Spec> {
    /// Sequencer information
    pub sequencer_info: TestSequencer<S>,
    /// Current sequence number of the sequencer
    current_sequence_number: Arc<AtomicU64>,
}

impl<S: Spec<Da = MockDaSpec>> TestPreferredSequencer<S> {
    /// Creates a new preferred test sequencer and initializes its sequence number.
    pub fn new(seq_info: TestSequencer<S>) -> Self {
        Self {
            sequencer_info: seq_info,
            current_sequence_number: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Builds a preferred batch. Increments the current sequence number
    pub fn build_preferred_batch<RT: Runtime<S>>(
        &self,
        batch: impl Into<BatchType<RT, S>>,
    ) -> SoftConfirmationBlobInfo<RT, S> {
        SoftConfirmationBlobInfo {
            batch_type: batch.into(),
            sequencer_address: self.sequencer_info.da_address,
            sequencer_info: SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: self
                    .current_sequence_number
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            },
        }
    }
}

/// A representation of a sequencer at genesis.
#[derive(Debug, Clone)]
pub struct TestSequencer<S: Spec> {
    /// The common user information.
    pub user_info: TestUser<S>,
    /// The DA address of the sequencer.
    pub da_address: <S::Da as DaSpec>::Address,
    /// The amount of tokens to bond at genesis. These tokens will be minted by the bank.
    pub bond: Amount,
}

impl<S: Spec> AsUser<S> for TestSequencer<S> {
    fn as_user(&self) -> &TestUser<S> {
        &self.user_info
    }

    fn as_user_mut(&mut self) -> &mut TestUser<S> {
        &mut self.user_info
    }
}

/// The configuration necessary to generate a [`TestSequencer`].
pub struct TestSequencerConfig<Da: DaSpec> {
    /// The additional balance of the sequencer on his bank account.
    pub additional_balance: Amount,
    /// The amount of tokens bonded by the sequencer.
    pub bond: Amount,
    /// The DA address of the sequencer.
    pub da_address: Da::Address,
}

impl<S: Spec> TestSequencer<S> {
    /// Generates a new [`TestSequencer`] with the given configuration.
    pub fn generate(config: TestSequencerConfig<S::Da>) -> Self {
        Self {
            user_info: TestUser::generate(config.additional_balance),
            da_address: config.da_address,
            bond: config.bond,
        }
    }
}
