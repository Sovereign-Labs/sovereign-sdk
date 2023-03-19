use super::{
    call::CallMessage,
    query::{QueryMessage, Response},
    types::Candidate,
    Election,
};

use crate::ADMIN;
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey, ZkMockContext},
    Context, Module, ModuleInfo, PublicKey,
};
use sov_state::{ProverStorage, WorkingSet, ZkStorage};

#[test]
fn test_election() {
    let native_storage = ProverStorage::temporary();
    let native_tx_store = WorkingSet::new(native_storage);
    test_module::<MockContext>(native_tx_store.clone());

    let (_log, witness) = native_tx_store.freeze();
    let zk_storage = ZkStorage::new([0u8; 32]);
    let zk_tx_store = WorkingSet::with_witness(zk_storage, witness);
    test_module::<ZkMockContext>(zk_tx_store);
}

fn test_module<C: Context<PublicKey = MockPublicKey>>(storage: WorkingSet<C::Storage>) {
    let admin_context = C::new(ADMIN);
    let ellection = &mut Election::<C>::new(storage);

    // Init module
    {
        ellection.genesis().unwrap();
    }

    // Send candidates
    {
        let set_candidates = CallMessage::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        ellection.call(set_candidates, &admin_context).unwrap();
    }

    let voter_1 = MockPublicKey::try_from("voter_1").unwrap().to_address();
    let voter_2 = MockPublicKey::try_from("voter_2").unwrap().to_address();
    let voter_3 = MockPublicKey::try_from("voter_3").unwrap().to_address();

    // Register voters
    {
        let add_voter = CallMessage::AddVoter(voter_1);
        ellection.call(add_voter, &admin_context).unwrap();

        let add_voter = CallMessage::AddVoter(voter_2);
        ellection.call(add_voter, &admin_context).unwrap();

        let add_voter = CallMessage::AddVoter(voter_3);
        ellection.call(add_voter, &admin_context).unwrap();
    }

    // Vote
    {
        let sender_context = C::new(voter_1);
        let vote = CallMessage::Vote(0);
        ellection.call(vote, &sender_context).unwrap();

        let sender_context = C::new(voter_2);
        let vote = CallMessage::Vote(1);
        ellection.call(vote, &sender_context).unwrap();

        let sender_context = C::new(voter_3);
        let vote = CallMessage::Vote(1);
        ellection.call(vote, &sender_context).unwrap();
    }

    ellection
        .call(CallMessage::FreezeElection, &admin_context)
        .unwrap();

    // Get result
    {
        let query = QueryMessage::GetResult;
        let query = ellection.query(query);
        let query_response: Response = serde_json::from_slice(&query.response).unwrap();

        assert_eq!(
            query_response,
            Response::Result(Some(Candidate {
                name: "candidate_2".to_owned(),
                count: 2
            }))
        )
    }
}
