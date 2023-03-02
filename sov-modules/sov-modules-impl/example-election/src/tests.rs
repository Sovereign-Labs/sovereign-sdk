use super::{
    call::CallMessage,
    query::{QueryMessage, Response},
    types::Candidate,
    Election,
};
use first_read_last_write_cache::cache::FirstReads;
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey, ZkMockContext},
    Context, Module, ModuleInfo,
};
use sov_state::{JmtStorage, ZkStorage};
use sovereign_db::state_db::StateDB;

#[test]
fn test_election() {
    let config = JmtStorage::temporary();
    test_module::<MockContext>(config.clone());

    let zk_config = ZkStorage::new(config.get_first_reads());
    test_module::<ZkMockContext>(zk_config);
}

fn test_module<C: Context<PublicKey = MockPublicKey>>(config: C::Config) {
    let admin = MockPublicKey::try_from("admin").unwrap();
    let admin_context = C::new(admin.clone(), config.clone());

    let ellection = &mut Election::<C>::new(admin_context.make_storage());

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

    let voter_1 = MockPublicKey::try_from("voter_1").unwrap();
    let voter_2 = MockPublicKey::try_from("voter_2").unwrap();
    let voter_3 = MockPublicKey::try_from("voter_3").unwrap();

    // Register voters
    {
        let add_voter = CallMessage::AddVoter(voter_1.clone());
        ellection.call(add_voter, &admin_context).unwrap();

        let add_voter = CallMessage::AddVoter(voter_2.clone());
        ellection.call(add_voter, &admin_context).unwrap();

        let add_voter = CallMessage::AddVoter(voter_3.clone());
        ellection.call(add_voter, &admin_context).unwrap();
    }

    // Vote
    {
        let sender_context = C::new(voter_1, config.clone());
        let vote = CallMessage::Vote(0);
        ellection.call(vote, &sender_context).unwrap();

        let sender_context = C::new(voter_2, config.clone());
        let vote = CallMessage::Vote(1);
        ellection.call(vote, &sender_context).unwrap();

        let sender_context = C::new(voter_3, config);
        let vote = CallMessage::Vote(1);
        ellection.call(vote, &sender_context).unwrap();
    }

    ellection
        .call(CallMessage::FreezeElection, &admin_context)
        .unwrap();

    // Get result
    {
        let query = QueryMessage::Result;
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
