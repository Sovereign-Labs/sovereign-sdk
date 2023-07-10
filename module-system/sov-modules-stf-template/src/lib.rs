pub mod app_template;
mod batch;
mod tx_verifier;

pub use app_template::AppTemplate;
pub use batch::Batch;
use sov_modules_api::hooks::{ApplyBlobHooks, TxHooks};
use sov_modules_api::{Context, DispatchCall, Genesis, Spec};
use sov_rollup_interface::stf::{BatchReceipt, StateTransitionFunction};
use sov_rollup_interface::zk::traits::Zkvm;
use sov_state::{StateCheckpoint, Storage};
pub use tx_verifier::RawTx;

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TxEffect {
    Reverted,
    Successful,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
// TODO: Should be generic for Address for pretty printing https://github.com/Sovereign-Labs/sovereign-sdk/issues/465
pub enum SequencerOutcome {
    /// Sequencer receives reward amount in defined token and can withdraw its deposit
    Rewarded(u64),
    /// Sequencer loses its deposit and receives no reward
    Slashed {
        reason: SlashingReason,
        // Keep this comment for so it doesn't need to investigate serde issue again.
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/465
        // #[serde(bound(deserialize = ""))]
        sequencer_da_address: Vec<u8>,
    },
    /// Batch was ignored, sequencer deposit left untouched.
    Ignored,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SlashingReason {
    InvalidBatchEncoding,
    StatelessVerificationFailed,
    InvalidTransactionEncoding,
}

impl<C: Context, RT, Vm: Zkvm> StateTransitionFunction<Vm> for AppTemplate<C, RT, Vm>
where
    RT: DispatchCall<Context = C>
        + Genesis<Context = C>
        + TxHooks<Context = C>
        + ApplyBlobHooks<Context = C, BlobResult = SequencerOutcome>,
{
    type StateRoot = jmt::RootHash;

    type InitialState = <RT as Genesis>::Config;

    type TxReceiptContents = TxEffect;

    type BatchReceiptContents = SequencerOutcome;

    type Witness = <<C as Spec>::Storage as Storage>::Witness;

    type MisbehaviorProof = ();

    fn init_chain(&mut self, params: Self::InitialState) {
        let mut working_set = StateCheckpoint::new(self.current_storage.clone()).to_revertable();

        self.runtime
            .genesis(&params, &mut working_set)
            .expect("module initialization must succeed");

        let (log, witness) = working_set.checkpoint().freeze();
        self.current_storage
            .validate_and_commit(log, &witness)
            .expect("Storage update must succeed");
    }

    fn begin_slot(&mut self, witness: Self::Witness) {
        self.checkpoint = Some(StateCheckpoint::with_witness(
            self.current_storage.clone(),
            witness,
        ));
    }

    fn apply_blob(
        &mut self,
        blob: &mut impl sov_rollup_interface::da::BlobTransactionTrait,
        _misbehavior_hint: Option<Self::MisbehaviorProof>,
    ) -> BatchReceipt<Self::BatchReceiptContents, Self::TxReceiptContents> {
        match self.apply_blob(blob) {
            Ok(batch) => batch,
            Err(e) => e.into(),
        }
    }

    fn end_slot(&mut self) -> (Self::StateRoot, Self::Witness) {
        let (cache_log, witness) = self.checkpoint.take().unwrap().freeze();
        let root_hash = self
            .current_storage
            .validate_and_commit(cache_log, &witness)
            .expect("jellyfish merkle tree update must succeed");
        (jmt::RootHash(root_hash), witness)
    }
}
