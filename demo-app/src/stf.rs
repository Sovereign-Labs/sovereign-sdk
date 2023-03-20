use crate::batch::Batch;
use std::cell::RefCell;

use crate::runtime::Runtime;
use crate::tx_verifier::{DemoAppTxVerifier, RawTx, TxVerifier};

use sov_modules_api::{Context, DispatchCall, Genesis};

use crate::tx_hooks::{DemoAppTxHooks, TxHooks};
use sov_state::{Storage, WorkingSet};
use sovereign_sdk::{
    core::{mocks::MockProof, traits::BatchTrait},
    jmt,
    stf::{ConsensusSetUpdate, OpaqueAddress, StateTransitionFunction},
};

pub(crate) struct Demo<C: Context, V: TxVerifier> {
    pub current_storage: C::Storage,
    pub verifier: V,
    pub working_set: RefCell<Option<WorkingSet<C::Storage>>>,
}

impl<C: Context> Demo<C, DemoAppTxVerifier<C>> {
    pub fn new(storage: C::Storage) -> Self {
        Self {
            current_storage: storage,
            verifier: DemoAppTxVerifier::new(),
            working_set: RefCell::new(None),
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
        let working_set = &mut WorkingSet::new(self.current_storage.clone());
        Runtime::<C>::genesis(working_set).expect("module initialization must succeed");
        let (log, witness) = working_set.freeze();
        self.current_storage
            .validate_and_commit(log, &witness)
            .expect("Storage update must succeed");
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
        let mut events = Vec::new();

        // Run the stateless verification.
        let txs = self
            .verifier
            .verify_txs_stateless(batch.take_transactions())
            .or(Err(ConsensusSetUpdate::slashing(sequencer)))?;
        let mut batch_workspace = WorkingSet::new(self.current_storage.clone());

        let mut tx_hooks = DemoAppTxHooks::<C>::new();

        for tx in txs {
            batch_workspace.to_revertable();
            // Run the stateful verification, possibly modifies the state.
            let verified_tx = tx_hooks
                .pre_dispatch_tx_hook(tx, &mut batch_workspace)
                .or(Err(ConsensusSetUpdate::slashing(sequencer)))?;

            if let Ok(msg) = Runtime::<C>::decode_call(&verified_tx.runtime_msg) {
                let ctx = C::new(verified_tx.sender);
                let tx_result = msg.dispatch_call(&mut batch_workspace, &ctx);

                tx_hooks.post_dispatch_tx_hook(verified_tx, &mut batch_workspace);

                match tx_result {
                    Ok(resp) => {
                        events.push(resp.events);
                        batch_workspace.commit();
                    }
                    Err(e) => {
                        // Don't merge the tx workspace. TODO add tests for this scenario
                        batch_workspace.revert();
                        panic!("Demo app txs must succeed but failed with err: {}", e)
                    }
                }
            } else {
                // If the serialization is invalid, the sequencer is malicious. Slash them
                batch_workspace.revert();
                return Err(ConsensusSetUpdate::slashing(sequencer));
            }
        }
        self.working_set.borrow_mut().replace(batch_workspace);

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
        let (cache_log, witness) = self.working_set.borrow_mut().take().unwrap().freeze();
        let root_hash = self
            .current_storage
            .validate_and_commit(cache_log, &witness)
            .expect("edree update must succed");
        (jmt::RootHash(root_hash), vec![])
    }
}
