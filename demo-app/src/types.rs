use crate::runtime::Runtime;
use borsh::{BorshDeserialize, BorshSerialize};

// Items that should go in prelude
use jmt::SimpleHasher;
use sov_modules_api::{
    mocks::{MockPublicKey, Transaction},
    Context, DispatchCall, Genesis,
};

use sov_state::Storage;
use sovereign_sdk::{
    core::{mocks::MockProof, traits::BatchTrait},
    jmt,
    stf::{ConsensusSetUpdate, OpaqueAddress, StateTransitionFunction},
};

pub struct Demo<C: Context> {
    pub current_storage: C::Storage,
}

impl<C: Context<PublicKey = MockPublicKey>> StateTransitionFunction for Demo<C> {
    type StateRoot = jmt::RootHash;

    type ChainParams = ();

    type Transaction = Transaction;

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
        for tx in batch.take_transactions() {
            // Do mock signature verification
            // We just check that the signature hash matches the tx hash
            let expected_hash = tx.mock_signature.msg_hash;
            let found_hash = C::Hasher::hash(&tx.msg);
            // If the (mock) signature is invalid, the sequencer is malicious. Slash them.
            if expected_hash != found_hash {
                return Err(ConsensusSetUpdate::slashing(sequencer));
            }

            if let Ok(msg) = Runtime::<C>::decode_call(&tx.msg) {
                let ctx = C::new(tx.mock_signature.pub_key);
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

#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct Batch {
    pub txs: Vec<Transaction>,
}

impl BatchTrait for Batch {
    type Transaction = Transaction;

    fn transactions(&self) -> &[Self::Transaction] {
        &self.txs
    }

    fn take_transactions(self) -> Vec<Self::Transaction> {
        self.txs
    }
}
