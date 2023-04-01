use crate::runtime::Runtime;
use crate::tx_verifier_impl::Transaction;

use borsh::BorshSerialize;
use sov_app_template::RawTx;
use sov_modules_api::mocks::{MockContext, MockPublicKey, MockSignature};
use sov_modules_api::PublicKey;

pub(crate) fn simulate_da() -> Vec<RawTx> {
    let mut messages = Vec::default();
    messages.extend(CallGenerator::election_call_messages());
    messages.extend(CallGenerator::value_setter_call_messages());
    messages
}

// Test helpers
struct CallGenerator {}

impl CallGenerator {
    fn election_call_messages() -> Vec<RawTx> {
        let mut messages = Vec::default();

        let admin = MockPublicKey::try_from("election_admin").unwrap();

        let set_candidates_message = election::call::CallMessage::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        let mut admin_nonce = 0;
        messages.push((admin.clone(), set_candidates_message, admin_nonce));

        let voters = vec![
            MockPublicKey::try_from("voter_1").unwrap(),
            MockPublicKey::try_from("voter_2").unwrap(),
            MockPublicKey::try_from("voter_3").unwrap(),
        ];

        for voter in voters {
            admin_nonce += 1;
            let add_voter_message = election::call::CallMessage::AddVoter(voter.to_address());

            messages.push((admin.clone(), add_voter_message, admin_nonce));

            let vote_message = election::call::CallMessage::Vote(1);
            messages.push((voter, vote_message, 0));
        }

        admin_nonce += 1;
        let freeze_message = election::call::CallMessage::FreezeElection;
        messages.push((admin, freeze_message, admin_nonce));

        messages
            .into_iter()
            .map(|(sender, m, nonce)| RawTx {
                data: Transaction::<MockContext>::new(
                    Runtime::<MockContext>::encode_election_call(m),
                    sender,
                    MockSignature::default(),
                    nonce,
                )
                .try_to_vec()
                .unwrap(),
            })
            .collect()
    }

    fn value_setter_call_messages() -> Vec<RawTx> {
        let admin = MockPublicKey::try_from("value_setter_admin").unwrap();
        let new_value = 99;

        let set_value_msg_1 =
            value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue { new_value });

        let new_value = 33;
        let set_value_msg_2 =
            value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue { new_value });

        vec![
            RawTx {
                data: Transaction::<MockContext>::new(
                    Runtime::<MockContext>::encode_value_setter_call(set_value_msg_1),
                    admin.clone(),
                    MockSignature::default(),
                    0,
                )
                .try_to_vec()
                .unwrap(),
            },
            RawTx {
                data: Transaction::<MockContext>::new(
                    Runtime::<MockContext>::encode_value_setter_call(set_value_msg_2),
                    admin,
                    MockSignature::default(),
                    1,
                )
                .try_to_vec()
                .unwrap(),
            },
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
