use borsh::{BorshDeserialize, BorshSerialize};
use example_election::Election;
// Items that should go in prelude
use jmt::SimpleHasher;
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Context, DispatchCall, DispatchQuery, Genesis, Module,
};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::JmtStorage;
use sovereign_sdk::{
    core::traits::{BatchTrait, BlockheaderTrait, CanonicalHash, TransactionTrait},
    jmt,
    serial::Encode,
};

// we should re-export anyhow from the modules_api

#[derive(BorshDeserialize, BorshSerialize, Debug, Clone, PartialEq, Eq)]
pub struct Header<C> {
    prev_hash: [u8; 32],
    tx_root: [u8; 32],
    phantom_ctx: std::marker::PhantomData<C>,
}

impl<C: Context> BlockheaderTrait for Header<C> {
    type Hash = [u8; 32];

    fn prev_hash(&self) -> &Self::Hash {
        &self.prev_hash
    }
}

impl<C: Context> CanonicalHash for Header<C> {
    type Output = [u8; 32];

    fn hash(&self) -> Self::Output {
        let mut hasher = C::Hasher::new();
        hasher.update(&self.prev_hash);
        hasher.update(&self.tx_root);
        hasher.finalize()
    }
}

#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct Batch<C: Context> {
    txs: Vec<RuntimeCall<C>>,
}

impl<C: Context> BatchTrait for Batch<C> {
    type Transaction = RuntimeCall<C>;

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

impl<C: Context> TransactionTrait for RuntimeCall<C> {
    type Hash = [u8; 32];
}

impl<C: Context> CanonicalHash for RuntimeCall<C> {
    type Output = [u8; 32];

    fn hash(&self) -> Self::Output {
        let serialized_message = self.encode_to_vec();
        C::Hasher::hash(serialized_message)
    }
}

fn main() {
    type RT = Runtime<MockContext>;
    let storage = JmtStorage::temporary();
    RT::genesis(storage.clone()).unwrap();

    let admin_key = MockPublicKey::try_from("admin").unwrap();
    let admin_context = MockContext::new(admin_key);

    // Election module
    // Send candidates
    {
        let set_candidates_message =
            example_election::call::CallMessage::<MockContext>::SetCandidates {
                names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
            };

        let serialized_message = RT::encode_election_call(set_candidates_message);
        let module = RT::decode_call(&serialized_message).unwrap();
        let result = module.dispatch_call(storage.clone(), &admin_context);
        assert!(result.is_ok())
    }

    let voters = vec![
        MockPublicKey::try_from("voter_1").unwrap(),
        MockPublicKey::try_from("voter_2").unwrap(),
        MockPublicKey::try_from("voter_3").unwrap(),
    ];

    // Add voters
    {
        for voter in voters.iter() {
            let add_voter_message =
                example_election::call::CallMessage::<MockContext>::AddVoter(voter.clone());

            let serialized_message = RT::encode_election_call(add_voter_message);
            let module = RT::decode_call(&serialized_message).unwrap();

            let result = module.dispatch_call(storage.clone(), &admin_context);
            assert!(result.is_ok())
        }
    }

    // Vote
    {
        for voter in voters {
            let voter_context = MockContext::new(voter);
            let vote_message = example_election::call::CallMessage::<MockContext>::Vote(1);

            let serialized_message = RT::encode_election_call(vote_message);
            let module = RT::decode_call(&serialized_message).unwrap();

            let result = module.dispatch_call(storage.clone(), &voter_context);
            assert!(result.is_ok())
        }
    }

    // Freeze
    {
        let freeze_message = example_election::call::CallMessage::<MockContext>::FreezeElection;

        let serialized_message = RT::encode_election_call(freeze_message);
        let module = RT::decode_call(&serialized_message).unwrap();

        let result = module.dispatch_call(storage.clone(), &admin_context);
        assert!(result.is_ok())
    }

    // Query the election module.
    {
        let query_message = example_election::query::QueryMessage::GetResult;

        let serialized_message = RT::encode_election_query(query_message);
        let module = RT::decode_query(&serialized_message).unwrap();

        let query_response = module.dispatch_query(storage.clone());

        let response: example_election::query::Response =
            serde_json::from_slice(&query_response.response).unwrap();

        assert_eq!(
            response,
            example_election::query::Response::Result(Some(example_election::Candidate {
                name: "candidate_2".to_owned(),
                count: 3
            }))
        )
    }

    println!("Hello, world!")
}
