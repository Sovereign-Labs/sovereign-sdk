use crate::batch::Batch;
use crate::runtime::Runtime;
use crate::tx_verifier::{DemoAppTxVerifier, RawTx, TxVerifier};

use sov_modules_api::{mocks::MockContext, Context, DispatchCall, Genesis, Spec};

use sov_state::Storage;
use sovereign_sdk::{
    core::{mocks::MockProof, traits::BatchTrait},
    jmt,
    stf::{ConsensusSetUpdate, OpaqueAddress, StateTransitionFunction},
};

pub(crate) struct Demo<C: Context, V: TxVerifier> {
    pub current_storage: C::Storage,
    pub verifier: V,
}

impl Demo<MockContext, DemoAppTxVerifier<MockContext>> {
    pub fn new(storage: <MockContext as Spec>::Storage) -> Self {
        Self {
            current_storage: storage,
            verifier: DemoAppTxVerifier::new(),
        }
    }
}

impl<C: Context, V> StateTransitionFunction for Demo<C, V>
where
    V: TxVerifier<Context = C>,
{
    type StateRoot = jmt::RootHash;

    type ChainParams = ();

    type Transaction = RawTx;

    type Batch = Batch;

    type Proof = MockProof;

    type MisbehaviorProof = ();

    fn init_chain(&mut self, _params: Self::ChainParams) {
        Runtime::<C>::genesis(self.current_storage.clone())
            .expect("module initialization must succeed");
        self.current_storage.finalize();
    }

    fn begin_slot(&self) {}

    fn apply_batch(
        &self,
        batch: Self::Batch,
        sequencer: &[u8],
        _misbehavior_hint: Option<Self::MisbehaviorProof>,
    ) -> Result<
        Vec<Vec<sovereign_sdk::stf::Event>>,
        sovereign_sdk::stf::ConsensusSetUpdate<OpaqueAddress>,
    > {
        let mut storage = self.current_storage.clone();
        let mut events = Vec::new();

        // Run the stateless verification.
        let txs = self
            .verifier
            .verify_txs_stateless(batch.take_transactions())
            .or(Err(ConsensusSetUpdate::slashing(sequencer)))?;

        for tx in txs {
            // Run the stateful verification, possibly modify the state.
            let verified_tx = self
                .verifier
                .verify_tx_stateful(tx)
                .or(Err(ConsensusSetUpdate::slashing(sequencer)))?;

            if let Ok(msg) = Runtime::<C>::decode_call(&verified_tx.runtime_msg) {
                let ctx = C::new(verified_tx.sender);
                let tx_result = msg.dispatch_call(storage.clone(), &ctx);

                match tx_result {
                    Ok(resp) => {
                        events.push(resp.events);
                        storage.merge();
                    }
                    Err(_) => {
                        // TODO add tests for this scenario
                        storage.merge_reads_and_discard_writes();
                    }
                }
            } else {
                // If the serialization is invalid, the sequencer is malicious. Slash them
                return Err(ConsensusSetUpdate::slashing(sequencer));
            }
        }

        Ok(events)
    }

    fn apply_proof(
        &self,
        _proof: Self::Proof,
        _prover: &[u8],
    ) -> Result<(), sovereign_sdk::stf::ConsensusSetUpdate<OpaqueAddress>> {
        todo!()
    }

    fn end_slot(
        &mut self,
    ) -> (
        Self::StateRoot,
        Vec<sovereign_sdk::stf::ConsensusSetUpdate<OpaqueAddress>>,
    ) {
        (jmt::RootHash(self.current_storage.finalize()), vec![])
    }
}
