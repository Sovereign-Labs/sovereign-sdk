use super::{
    call::CallMessage,
    query::{GetResultResponse, QueryMessage},
    types::Candidate,
    Election,
};

use anyhow::anyhow;
use sov_modules_api::{
    default_context::{DefaultContext, ZkDefaultContext},
    default_signature::DefaultPublicKey,
    Context, Module, ModuleInfo, PublicKey,
};
use sov_state::{ProverStorage, WorkingSet, ZkStorage};

#[test]
fn test_election() {
    let native_storage = ProverStorage::temporary();
    let mut native_working_set = WorkingSet::new(native_storage);
    test_module::<DefaultContext>(&mut native_working_set);

    let (_log, witness) = native_working_set.freeze();
    let zk_storage = ZkStorage::new([0u8; 32]);
    let mut zk_working_set = WorkingSet::with_witness(zk_storage, witness);
    test_module::<ZkDefaultContext>(&mut zk_working_set);
}

fn test_module<C: Context<PublicKey = DefaultPublicKey>>(working_set: &mut WorkingSet<C::Storage>) {
    let admin_pub_key = C::PublicKey::try_from("election_admin")
        .map_err(|_| anyhow!("Admin initialization failed"))
        .unwrap();

    let admin_context = C::new(admin_pub_key.to_address());
    let election = &mut Election::<C>::new();

    // Init module
    {
        election.genesis(&(), working_set).unwrap();
    }

    // Send candidates
    {
        let set_candidates = CallMessage::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        election
            .call(set_candidates, &admin_context, working_set)
            .unwrap();
    }

    let voter_1 = DefaultPublicKey::from("voter_1").to_address::<C::Address>();
    let voter_2 = DefaultPublicKey::from("voter_2").to_address::<C::Address>();
    let voter_3 = DefaultPublicKey::from("voter_3").to_address::<C::Address>();

    // Register voters
    {
        let add_voter = CallMessage::AddVoter(voter_1.clone());
        election
            .call(add_voter, &admin_context, working_set)
            .unwrap();

        let add_voter = CallMessage::AddVoter(voter_2.clone());
        election
            .call(add_voter, &admin_context, working_set)
            .unwrap();

        let add_voter = CallMessage::AddVoter(voter_3.clone());
        election
            .call(add_voter, &admin_context, working_set)
            .unwrap();
    }

    // Vote
    {
        let sender_context = C::new(voter_1);
        let vote = CallMessage::Vote(0);
        election.call(vote, &sender_context, working_set).unwrap();

        let sender_context = C::new(voter_2);
        let vote = CallMessage::Vote(1);
        election.call(vote, &sender_context, working_set).unwrap();

        let sender_context = C::new(voter_3);
        let vote = CallMessage::Vote(1);
        election.call(vote, &sender_context, working_set).unwrap();
    }

    election
        .call(CallMessage::FreezeElection, &admin_context, working_set)
        .unwrap();

    // Get result
    {
        let query = QueryMessage::GetResult;
        let query = election.query(query, working_set);
        let query_response: GetResultResponse = serde_json::from_slice(&query.response).unwrap();

        assert_eq!(
            query_response,
            GetResultResponse::Result(Some(Candidate {
                name: "candidate_2".to_owned(),
                count: 2
            }))
        )
    }
}
