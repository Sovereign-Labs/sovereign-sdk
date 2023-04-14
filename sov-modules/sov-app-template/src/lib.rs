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

pub struct AppTemplate<C: Context, V, RT, H, GenesisConfig> {
    pub current_storage: C::Storage,
    pub runtime: RT,
    tx_verifier: V,
    tx_hooks: H,
    genesis_config: GenesisConfig,
    working_set: Option<WorkingSet<C::Storage>>,
}

impl<C: Context, V, RT, H, GenesisConfig> AppTemplate<C, V, RT, H, GenesisConfig>
where
    RT: DispatchCall<Context = C> + Genesis<Context = C, Config = GenesisConfig>,
    V: TxVerifier,
    H: TxHooks<Context = C, Transaction = <V as TxVerifier>::Transaction>,
{
    pub fn new(
        storage: C::Storage,
        runtime: RT,
        tx_verifier: V,
        tx_hooks: H,
        genesis_config: GenesisConfig,
    ) -> Self {
        Self {
            runtime,
            current_storage: storage,
            tx_verifier,
            tx_hooks,
            genesis_config,
            working_set: None,
        }
    }

    fn slash(&mut self, mut batch_workspace: WorkingSet<C::Storage>) {
        // It is ok to unwrap here, we are sure that sequencer has enough funds locked.
        self.tx_hooks.slash_sequencer(&mut batch_workspace).unwrap();
        // TODO: should we save the working_set here?
        self.working_set = Some(batch_workspace);
    }
}

impl<C: Context, V, RT, H, GenesisConfig> StateTransitionFunction
    for AppTemplate<C, V, RT, H, GenesisConfig>
where
    RT: DispatchCall<Context = C> + Genesis<Context = C, Config = GenesisConfig>,
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
            .genesis(&self.genesis_config, working_set)
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
        // Question: Should we return an enum error here? (see TODOs bellow)
        sovereign_sdk::stf::ConsensusSetUpdate<OpaqueAddress>,
    > {
        let mut batch_workspace = WorkingSet::new(self.current_storage.clone());
        batch_workspace = batch_workspace.to_revertable();

        // TODO: Handle errors
        match self.tx_hooks.next_sequencer(&mut batch_workspace) {
            Ok(next_sequencer) => {
                if next_sequencer != sequencer {
                    // TODO: Return an error, should we slash in this case?
                    todo!()
                }
            }
            // TODO: return an error if sequencer doesn't exist
            Err(_) => todo!(),
        }

        // TODO: Handle an error (sequencer doesn't have enough funds)
        match self.tx_hooks.lock_sequencer_funds(&mut batch_workspace) {
            Ok(_) => {}
            Err(_) => todo!(),
        }

        let mut events = Vec::new();

        // Run the stateless verification.
        let txs = match self
            .tx_verifier
            .verify_txs_stateless(batch.take_transactions())
        {
            Ok(txs) => txs,
            Err(_) => {
                // It is ok to not revert here, `verify_txs_stateless` won't modify the state
                // but the sequencer is slashed
                self.slash(batch_workspace);
                return Err(ConsensusSetUpdate::slashing(sequencer));
            }
        };

        for tx in txs {
            batch_workspace = batch_workspace.to_revertable();
            // Run the stateful verification, possibly modifies the state.
            let verified_tx = match self.tx_hooks.pre_dispatch_tx_hook(tx, &mut batch_workspace) {
                Ok(verified_tx) => verified_tx,
                Err(_) => {
                    // TODO: We should revert here but somehow preserve the slashing.
                    self.slash(batch_workspace);
                    return Err(ConsensusSetUpdate::slashing(sequencer));
                }
            };

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
                        // TODO: figure out how to handle slashing & reverts.
                        let batch_workspace = batch_workspace.revert();

                        // Maybe something like:
                        /*
                        let batch_workspace = batch_workspace.revert();
                        let mut batch_workspace = batch_workspace.to_revertable();
                        // we locks funds again
                        self.tx_hooks
                            .lock_sequencer_funds(&mut batch_workspace)
                            .unwrap();
                        self.working_set = Some(batch_workspace);
                        */

                        panic!("Demo app txs must succeed but failed with err: {}", e)
                    }
                }
            } else {
                // If the serialization is invalid, the sequencer is malicious. Slash them.\
                // TODO: We don't revert is that ok?
                self.slash(batch_workspace);
                return Err(ConsensusSetUpdate::slashing(sequencer));
            }
        }

        // TODO:
        // - `reward_sequencer` shouldn't fail so unwrapping here is ok.
        // - calculate the amount based of gas and fees
        self.tx_hooks
            .reward_sequencer(0, &mut batch_workspace)
            .unwrap();

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
        // TODO: maybe we should apply slashing here and not in  `apply_batch`:
        // 1. Apply batch locks sequencer funds
        // 2. Sequencer did something wrong all the changes are reverted (sequencer funds are not locked anymore), but we propagate `is_reverted` flag.
        // 3. in `end_slot` we do the slashing.

        let (cache_log, witness) = self.working_set.take().unwrap().freeze();
        let root_hash = self
            .current_storage
            .validate_and_commit(cache_log, &witness)
            .expect("edree update must succed");
        (jmt::RootHash(root_hash), vec![])
    }
}
