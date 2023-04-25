use crate::runtime::Runtime;
use crate::tx_verifier_impl::Transaction;

use borsh::BorshSerialize;
use sov_app_template::RawTx;
use sov_modules_api::mocks::{MockContext, MockPublicKey, MockSignature};
use sov_modules_api::PublicKey;

pub(crate) fn simulate_da() -> Vec<RawTx> {
    let call_generator = &mut CallGenerator::new();
    let mut messages = Vec::default();
    messages.extend(call_generator.election_call_messages());
    messages.extend(call_generator.value_setter_call_messages());
    messages
}

pub(crate) fn simulate_da_with_revert_msg() -> Vec<RawTx> {
    let call_generator = &mut CallGenerator::new();
    call_generator.election_call_messages_with_revert()
}

pub(crate) fn simulate_da_with_bad_sig() -> Vec<RawTx> {
    let call_generator = &mut CallGenerator::new();
    call_generator.election_call_messages_bad_sig()
}

// Test helpers

struct CallGenerator {
    election_admin_nonce: u64,
    election_admin: MockPublicKey,
    value_setter_admin_nonce: u64,
    value_setter_admin: MockPublicKey,
}

impl CallGenerator {
    fn new() -> Self {
        Self {
            election_admin_nonce: 0,
            election_admin: MockPublicKey::try_from("election_admin").unwrap(),
            value_setter_admin_nonce: 0,
            value_setter_admin: MockPublicKey::try_from("value_setter_admin").unwrap(),
        }
    }

    fn create_voters_and_vote(
        &mut self,
    ) -> Vec<(MockPublicKey, election::call::CallMessage<MockContext>, u64)> {
        let mut messages = Vec::default();

        let set_candidates_message = election::call::CallMessage::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        messages.push((
            self.election_admin.clone(),
            set_candidates_message,
            self.election_admin_nonce,
        ));
        self.election_admin_nonce += 1;

        let voters = vec![
            MockPublicKey::try_from("voter_1").unwrap(),
            MockPublicKey::try_from("voter_2").unwrap(),
            MockPublicKey::try_from("voter_3").unwrap(),
        ];

        for voter in voters {
            let add_voter_message = election::call::CallMessage::AddVoter(voter.to_address());

            messages.push((
                self.election_admin.clone(),
                add_voter_message,
                self.election_admin_nonce,
            ));

            let vote_message = election::call::CallMessage::Vote(1);
            messages.push((voter, vote_message, 0));
            self.election_admin_nonce += 1;
        }

        messages
    }

    fn freeze_vote(
        &mut self,
    ) -> Vec<(MockPublicKey, election::call::CallMessage<MockContext>, u64)> {
        let mut messages = Vec::default();

        let freeze_message = election::call::CallMessage::FreezeElection;
        messages.push((
            self.election_admin.clone(),
            freeze_message,
            self.election_admin_nonce,
        ));
        self.election_admin_nonce += 1;

        messages
    }

    fn election_call_messages(&mut self) -> Vec<RawTx> {
        let mut messages = Vec::default();

        {
            let create_voter_messages = self.create_voters_and_vote();
            messages.extend(create_voter_messages.into_iter());
        }

        {
            let freeze_vote_messages = self.freeze_vote();
            messages.extend(freeze_vote_messages.into_iter());
        }

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

    fn election_call_messages_with_revert(&mut self) -> Vec<RawTx> {
        let mut messages = Vec::default();
        let create_voter_messages = self.create_voters_and_vote();
        messages.extend(create_voter_messages.into_iter());

        // Invalid message: This voter already voted.
        {
            let voter = MockPublicKey::try_from("voter_1").unwrap();
            let vote_message = election::call::CallMessage::Vote(1);
            messages.push((voter, vote_message, 1));
        }

        {
            let freeze_vote_messages = self.freeze_vote();
            messages.extend(freeze_vote_messages.into_iter());
        }
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

    fn election_call_messages_bad_sig(&mut self) -> Vec<RawTx> {
        let mut messages = Vec::default();
        messages.extend(self.create_voters_and_vote());
        messages.extend(self.freeze_vote());

        let mut messages_iter = messages.into_iter().peekable();

        let mut serialized_messages = Vec::default();
        while let Some((sender, m, nonce)) = messages_iter.next() {
            // The last message has bad signature.
            let should_fail = messages_iter.peek().is_none();

            serialized_messages.push(RawTx {
                data: Transaction::<MockContext>::new(
                    Runtime::<MockContext>::encode_election_call(m),
                    sender,
                    MockSignature {
                        msg_sig: Vec::default(),
                        should_fail,
                    },
                    nonce,
                )
                .try_to_vec()
                .unwrap(),
            });
        }

        serialized_messages
    }

    fn value_setter_call_messages(&mut self) -> Vec<RawTx> {
        let mut messages = Vec::default();

        let new_value = 99;

        let set_value_msg_1 =
            value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue { new_value });

        let new_value = 33;
        let set_value_msg_2 =
            value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue { new_value });

        messages.push(RawTx {
            data: Transaction::<MockContext>::new(
                Runtime::<MockContext>::encode_value_setter_call(set_value_msg_1),
                self.value_setter_admin.clone(),
                MockSignature::default(),
                self.value_setter_admin_nonce,
            )
            .try_to_vec()
            .unwrap(),
        });

        self.value_setter_admin_nonce += 1;
        messages.push(RawTx {
            data: Transaction::<MockContext>::new(
                Runtime::<MockContext>::encode_value_setter_call(set_value_msg_2),
                self.value_setter_admin.clone(),
                MockSignature::default(),
                self.value_setter_admin_nonce,
            )
            .try_to_vec()
            .unwrap(),
        });

        messages
    }
}

pub(crate) struct QueryGenerator {}

impl QueryGenerator {
    pub(crate) fn generate_query_election_message() -> Vec<u8> {
        let query_message = election::query::QueryMessage::GetResult;
        Runtime::<MockContext>::encode_election_query(query_message)
    }

    pub(crate) fn generate_query_election_nb_of_votes_message() -> Vec<u8> {
        let query_message = election::query::QueryMessage::GenNbOfVotes;
        Runtime::<MockContext>::encode_election_query(query_message)
    }

    pub(crate) fn generate_query_value_setter_message() -> Vec<u8> {
        let query_message = value_setter::query::QueryMessage::GetValue;
        Runtime::<MockContext>::encode_value_setter_query(query_message)
    }
}
