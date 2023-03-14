use crate::runtime::Runtime;
use borsh::{BorshDeserialize, BorshSerialize};

use first_read_last_write_cache::cache::FirstReads;
// Items that should go in prelude
use jmt::SimpleHasher;
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey, Transaction, ZkMockContext},
    Context, DispatchCall, Genesis, Spec,
};

use sov_state::Storage;
use sovereign_sdk::{
    core::{mocks::MockProof, traits::BatchTrait},
    jmt,
    stf::{ConsensusSetUpdate, OpaqueAddress, StateTransitionFunction},
};

pub trait ExecutionWitness<C: Context> {
    fn update(&mut self, storage: C::Storage) -> jmt::RootHash;
}

pub struct NativeWitness {
    pub first_reads: Option<FirstReads>,
    //TODO add hints
}

impl ExecutionWitness<MockContext> for NativeWitness {
    fn update(&mut self, mut storage: <MockContext as Spec>::Storage) -> jmt::RootHash {
        let reads = storage.get_first_reads();

        self.first_reads = Some(reads);
        jmt::RootHash(storage.finalize())
    }
}

pub struct ZkWitness {}

impl ExecutionWitness<ZkMockContext> for NativeWitness {
    fn update(&mut self, mut storage: <ZkMockContext as Spec>::Storage) -> jmt::RootHash {
        jmt::RootHash(storage.finalize())
    }
}

pub struct Demo<C: Context, SE: ExecutionWitness<C>> {
    pub current_storage: C::Storage,
    pub witness: SE,
}

impl<C: Context<PublicKey = MockPublicKey>, SE: ExecutionWitness<C>> StateTransitionFunction
    for Demo<C, SE>
{
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
        let root = self.witness.update(self.current_storage.clone());
        (root, vec![])
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
