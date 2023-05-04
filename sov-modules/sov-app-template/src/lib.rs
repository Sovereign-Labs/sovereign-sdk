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
    stf::{OpaqueAddress, StateTransitionFunction},
};

pub struct AppTemplate<C: Context, V, RT, H> {
    pub current_storage: C::Storage,
    pub runtime: RT,
    tx_verifier: V,
    tx_hooks: H,
    working_set: Option<WorkingSet<C::Storage>>,
}

impl<C: Context, V, RT, H> AppTemplate<C, V, RT, H>
where
    RT: DispatchCall<Context = C> + Genesis<Context = C>,
    V: TxVerifier,
    H: TxHooks<Context = C, Transaction = <V as TxVerifier>::Transaction>,
{
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

    type InitialState = <RT as Genesis>::Config;

    type Transaction = RawTx;

    type Batch = Batch;

    type Proof = MockProof;

    type MisbehaviorProof = ();

    fn init_chain(&mut self, params: Self::InitialState) {
        let working_set = &mut WorkingSet::new(self.current_storage.clone());

        self.runtime
            .genesis(&params, working_set)
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
    ) -> anyhow::Result<Vec<Vec<sovereign_sdk::stf::Event>>> {
        let mut batch_workspace = WorkingSet::new(self.current_storage.clone());
        batch_workspace = batch_workspace.to_revertable();

        if let Err(e) = self
            .tx_hooks
            .enter_apply_batch(sequencer, &mut batch_workspace)
        {
            anyhow::bail!(
                "Error: The transaction was rejected by the 'enter_apply_batch' hook. {}",
                e
            )
        }

        // Commit `enter_apply_batch` changes.
        batch_workspace = batch_workspace.commit().to_revertable();

        let mut events = Vec::new();

        // Run the stateless verification, since it is stateless we don't commit.
        let txs = match self
            .tx_verifier
            .verify_txs_stateless(batch.take_transactions())
        {
            Ok(txs) => txs,
            Err(e) => {
                // Revert on error
                let batch_workspace = batch_workspace.revert();
                self.working_set = Some(batch_workspace);
                anyhow::bail!("Stateless verification error - the sequencer included a transaction which was known to be invalid. {}", e);
            }
        };

        // Process transactions in a loop, commit changes after every step of the loop.
        for tx in txs {
            batch_workspace = batch_workspace.to_revertable();
            // Run the stateful verification, possibly modifies the state.
            let verified_tx = match self.tx_hooks.pre_dispatch_tx_hook(tx, &mut batch_workspace) {
                Ok(verified_tx) => verified_tx,
                Err(e) => {
                    // Revert the batch.
                    batch_workspace = batch_workspace.revert();

                    // We reward sequencer funds inside `exit_apply_batch`.
                    self.tx_hooks
                        .exit_apply_batch(0, &mut batch_workspace)
                        .expect("Impossible happened: error in exit_apply_batch");

                    self.working_set = Some(batch_workspace);
                    anyhow::bail!("Stateful verification error - the sequencer included an invalid transaction: {}", e);
                }
            };

            match RT::decode_call(verified_tx.runtime_message()) {
                Ok(msg) => {
                    let ctx = C::new(verified_tx.sender().clone());
                    let tx_result = self.runtime.dispatch_call(msg, &mut batch_workspace, &ctx);

                    self.tx_hooks
                        .post_dispatch_tx_hook(verified_tx, &mut batch_workspace);

                    match tx_result {
                        Ok(resp) => {
                            events.push(resp.events);
                        }
                        Err(_e) => {
                            // The transaction causing invalid state transition is reverted but we don't slash and we continue
                            // processing remaining transactions.
                            batch_workspace = batch_workspace.revert();
                        }
                    }
                }
                Err(e) => {
                    // If the serialization is invalid, the sequencer is malicious. Slash them (we don't run exit_apply_batch here)
                    let batch_workspace = batch_workspace.revert();
                    self.working_set = Some(batch_workspace);
                    anyhow::bail!("Tx decoding error: {}", e);
                }
            }
            // commit each step of the loop
            batch_workspace = batch_workspace.commit();
        }

        // TODO: calculate the amount based of gas and fees
        self.tx_hooks
            .exit_apply_batch(0, &mut batch_workspace)
            .expect("Impossible happened: error in exit_apply_batch");

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
            .validate_and_commit(&cache_log, &witness)
            .expect("jellyfish merkle tree update must succeed");

        (jmt::RootHash(root_hash), vec![])
    }
}
