use sov_modules_api::mocks::{MockContext, MockPublicKey, Transaction};

use crate::runtime::Runtime;

pub(crate) fn simulate_da() -> Vec<Transaction> {
    let mut messages = Vec::default();
    messages.extend(CallGenerator::election_call_messages());
    messages.extend(CallGenerator::value_setter_call_messages());
    messages
}

// Test helpers
struct CallGenerator {}

impl CallGenerator {
    fn election_call_messages() -> Vec<Transaction> {
        let mut messages = Vec::default();

        let admin = MockPublicKey::try_from("admin").unwrap();

        let set_candidates_message = election::call::CallMessage::<MockContext>::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        messages.push((admin.clone(), set_candidates_message));

        let voters = vec![
            MockPublicKey::try_from("voter_1").unwrap(),
            MockPublicKey::try_from("voter_2").unwrap(),
            MockPublicKey::try_from("voter_3").unwrap(),
        ];

        for voter in voters {
            let add_voter_message =
                election::call::CallMessage::<MockContext>::AddVoter(voter.clone());

            messages.push((admin.clone(), add_voter_message));

            let vote_message = election::call::CallMessage::<MockContext>::Vote(1);
            messages.push((voter, vote_message));
        }

        let freeze_message = election::call::CallMessage::<MockContext>::FreezeElection;
        messages.push((admin, freeze_message));

        messages
            .into_iter()
            .map(|(sender, m)| {
                Transaction::new(Runtime::<MockContext>::encode_election_call(m), sender)
            })
            .collect()
    }

    fn value_setter_call_messages() -> Vec<Transaction> {
        let admin = MockPublicKey::try_from("admin").unwrap();
        let new_value = 99;

        let set_value_msg_1 =
            value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue { new_value });

        let new_value = 33;
        let set_value_msg_2 =
            value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue { new_value });

        vec![
            Transaction::new(
                Runtime::<MockContext>::encode_value_setter_call(set_value_msg_1),
                admin.clone(),
            ),
            Transaction::new(
                Runtime::<MockContext>::encode_value_setter_call(set_value_msg_2),
                admin,
            ),
        ]
    }
}

pub(crate) struct QueryGenerator {}

impl QueryGenerator {
    pub(crate) fn generate_query_election_message() -> Vec<u8> {
        let query_message = election::query::QueryMessage::GetResult;
        Runtime::<MockContext>::encode_election_query(query_message)
    }

    pub(crate) fn generate_query_value_setter_message() -> Vec<u8> {
        let query_message = value_setter::query::QueryMessage::GetValue;
        Runtime::<MockContext>::encode_value_setter_query(query_message)
    }
}
