mod batch;
mod tx_hooks;
mod tx_verifier;

pub use batch::Batch;
pub use tx_hooks::TxHooks;
pub use tx_hooks::VerifiedTx;
pub use tx_verifier::{RawTx, TxVerifier};

use sov_modules_api::{Context, DispatchCall, Genesis};
use sov_state::{Storage, WorkingSet};
use sovereign_sdk::{
    core::{mocks::MockProof, traits::BatchTrait},
    jmt,
    stf::{ConsensusSetUpdate, OpaqueAddress, StateTransitionFunction},
};

pub struct AppTemplate<C: Context, V, RT, H> {
    pub current_storage: C::Storage,
    pub runtime: RT,
    tx_verifier: V,
    tx_hooks: H,
    working_set: Option<WorkingSet<C::Storage>>,
}

impl<C: Context, V, RT, H> AppTemplate<C, V, RT, H> {
    pub fn new(storage: C::Storage, runtime: RT, tx_verifier: V, tx_hooks: H) -> Self {
        Self {
            runtime,
            current_storage: storage,
            tx_verifier,
            tx_hooks,
            working_set: None,
        }
    }
}

impl<C: Context, V, RT, H> StateTransitionFunction for AppTemplate<C, V, RT, H>
where
    RT: DispatchCall<Context = C> + Genesis<Context = C>,
    V: TxVerifier,
    H: TxHooks<Context = C, Transaction = <V as TxVerifier>::Transaction>,
{
    type StateRoot = jmt::RootHash;

    type ChainParams = ();

    type Transaction = RawTx;

    type Batch = Batch;

    type Proof = MockProof;

    type MisbehaviorProof = ();

    fn init_chain(&mut self, _params: Self::ChainParams) {
        let working_set = &mut WorkingSet::new(self.current_storage.clone());
        self.runtime
            .genesis(working_set)
            .expect("module initialization must succeed");
        let (log, witness) = working_set.freeze();
        self.current_storage
            .validate_and_commit(log, &witness)
            .expect("Storage update must succeed");
    }

    fn begin_slot(&mut self) {
        self.working_set = Some(WorkingSet::new(self.current_storage.clone()));
    }

    fn apply_batch(
        &mut self,
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
            .tx_verifier
            .verify_txs_stateless(batch.take_transactions())
            .or(Err(ConsensusSetUpdate::slashing(sequencer)))?;
        let mut batch_workspace = WorkingSet::new(self.current_storage.clone());

        for tx in txs {
            batch_workspace = batch_workspace.to_revertable();
            // Run the stateful verification, possibly modifies the state.
            let verified_tx = self
                .tx_hooks
                .pre_dispatch_tx_hook(tx, &mut batch_workspace)
                .or(Err(ConsensusSetUpdate::slashing(sequencer)))?;

            if let Ok(msg) = RT::decode_call(verified_tx.runtime_message()) {
                let ctx = C::new(verified_tx.sender().clone());
                let tx_result = self.runtime.dispatch_call(msg, &mut batch_workspace, &ctx);

                self.tx_hooks
                    .post_dispatch_tx_hook(verified_tx, &mut batch_workspace);

                match tx_result {
                    Ok(resp) => {
                        events.push(resp.events);
                        batch_workspace = batch_workspace.commit();
                    }
                    Err(e) => {
                        // Don't merge the tx workspace. TODO add tests for this scenario
                        batch_workspace.revert();
                        panic!("Demo app txs must succeed but failed with err: {}", e)
                    }
                }
            } else {
                // If the serialization is invalid, the sequencer is malicious. Slash them.
                batch_workspace.revert();
                return Err(ConsensusSetUpdate::slashing(sequencer));
            }
        }
        self.working_set = Some(batch_workspace);

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
        let (cache_log, witness) = self.working_set.take().unwrap().freeze();
        let root_hash = self
            .current_storage
            .validate_and_commit(cache_log, &witness)
            .expect("edree update must succed");
        (jmt::RootHash(root_hash), vec![])
    }
}
