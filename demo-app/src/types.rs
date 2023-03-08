use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use example_election::Election;
// Items that should go in prelude
use jmt::SimpleHasher;
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Context, DispatchCall, Genesis, Module, Spec,
};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::{JmtStorage, Storage};
use sovereign_sdk::{
    core::{
        mocks::MockProof,
        traits::{BatchTrait, CanonicalHash, TransactionTrait},
    },
    jmt,
    stf::{ConsensusSetUpdate, OpaqueAddress, StateTransitionFunction},
};

// we should re-export anyhow from the modules_api

// TODO: Unify with Message from basic_runtime.rs
#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize)]
pub struct DemoTransaction<C: Context> {
    mock_signature: MockSignature,
    msg: Vec<u8>,
    phantom_context: PhantomData<C>,
}

// TODO: unify with MockSignature from sov-modules-api mocks
#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize)]
pub struct MockSignature {
    pub_key: MockPublicKey,
    msg_hash: [u8; 32],
}

pub struct Demo {
    pub current_storage: JmtStorage,
}

impl StateTransitionFunction for Demo {
    type StateRoot = jmt::RootHash;

    type ChainParams = ();

    type Transaction = DemoTransaction<MockContext>;

    type Batch = Batch<MockContext>;

    type Proof = MockProof;

    type MisbehaviorProof = ();

    fn init_chain(&mut self, _params: Self::ChainParams) {
        Runtime::<MockContext>::genesis(self.current_storage.clone())
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
            let found_hash = <MockContext as Spec>::Hasher::hash(&tx.msg);
            // If the (mock) signature is invalid, the sequencer is malicious. Slash them.
            if expected_hash != found_hash {
                return Err(ConsensusSetUpdate::slashing(sequencer));
            }

            if let Ok(msg) = Runtime::<MockContext>::decode_call(&tx.msg) {
                let ctx = MockContext::new(tx.mock_signature.pub_key);
                let tx_result = msg.dispatch_call(storage.clone(), &ctx);
                match tx_result {
                    Ok(resp) => {
                        events.push(resp.events);
                        storage.merge();
                    }
                    Err(_) => {
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
        self.current_storage.finalize();
        todo!()
    }
}

#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct Batch<C: Context> {
    pub txs: Vec<DemoTransaction<C>>,
}

impl<C: Context> BatchTrait for Batch<C> {
    type Transaction = DemoTransaction<C>;

    fn transactions(&self) -> &[Self::Transaction] {
        &self.txs
    }

    fn take_transactions(self) -> Vec<Self::Transaction> {
        self.txs
    }
}

#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
pub struct Runtime<C: Context> {
    #[allow(unused)]
    election: Election<C>,
}

impl<C: Context> core::fmt::Debug for Runtime<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").finish()
    }
}

impl<C: Context> TransactionTrait for DemoTransaction<C> {
    type Hash = [u8; 32];
}

impl<C: Context> CanonicalHash for DemoTransaction<C> {
    type Output = [u8; 32];

    fn hash(&self) -> Self::Output {
        self.mock_signature.msg_hash.clone()
    }
}
