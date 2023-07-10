use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Address, Context, Module, PublicKey};
use sov_state::{ProverStorage, WorkingSet, ZkStorage};

use super::call::CallMessage;
use super::query::GetResultResponse;
use super::types::Candidate;
use super::Election;
use crate::ElectionConfig;

#[test]
fn test_election() {
    let admin = Address::from([1; 32]);

    let tmpdir = tempfile::tempdir().unwrap();
    let native_storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut native_working_set = WorkingSet::new(native_storage);

    test_module::<DefaultContext>(admin.clone(), &mut native_working_set);

    let (_log, witness) = native_working_set.checkpoint().freeze();
    let zk_storage = ZkStorage::new([0u8; 32]);
    let mut zk_working_set = WorkingSet::with_witness(zk_storage, witness);
    test_module::<ZkDefaultContext>(admin, &mut zk_working_set);
}

fn test_module<C: Context>(admin: C::Address, working_set: &mut WorkingSet<C::Storage>) {
    let admin_context = C::new(admin.clone());
    let election = &mut Election::<C>::default();

    // Init module
    {
        let config = ElectionConfig { admin };
        election.genesis(&config, working_set).unwrap();
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

    let voter_1 = DefaultPrivateKey::generate()
        .pub_key()
        .to_address::<C::Address>();

    let voter_2 = DefaultPrivateKey::generate()
        .pub_key()
        .to_address::<C::Address>();

    let voter_3 = DefaultPrivateKey::generate()
        .pub_key()
        .to_address::<C::Address>();

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
        let query_response: GetResultResponse = election.results(working_set);

        assert_eq!(
            query_response,
            GetResultResponse::Result(Some(Candidate {
                name: "candidate_2".to_owned(),
                count: 2
            }))
        )
    }
}
