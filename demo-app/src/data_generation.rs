use crate::runtime::Runtime;
use crate::tx_verifier_impl::Transaction;

use borsh::BorshSerialize;

use sov_app_template::RawTx;
use sov_modules_api::mocks::{MockContext, MockPublicKey, MockSignature};
use sov_modules_api::PublicKey;

pub(crate) fn simulate_da() -> Vec<RawTx> {
    let em = election_messages::ElectionCallMessages {};

    let mut messages = Vec::default();
    messages.extend(em.create_raw_txs());

    let call_generator = value_setter_messages::CallGenerator {
        value_setter_admin: MockPublicKey::try_from("value_setter_admin").unwrap(),
    };

    let vm = value_setter_messages::ValueSetterMessages { call_generator };

    messages.extend(vm.create_raw_txs());
    messages
}

pub(crate) fn simulate_da_with_revert_msg() -> Vec<RawTx> {
    let em = election_messages::InvalidElectionCallMessages {};
    em.create_raw_txs()
}

pub(crate) fn simulate_da_with_bad_sig() -> Vec<RawTx> {
    let em = election_messages::BadSigElectionCallMessages {};
    em.create_raw_txs()
}

pub(crate) fn simulate_da_with_bad_nonce() -> Vec<RawTx> {
    let em = election_messages::BadNonceElectionCallMessages {};
    em.create_raw_txs()
}

pub(crate) fn simulate_da_with_bad_serialization() -> Vec<RawTx> {
    let em = election_messages::BadSerializationElectionCallMessages {};
    em.create_raw_txs()
}

trait MessageGenerator {
    type Call;

    fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)>;

    fn create_txs(
        &self,
        sender: MockPublicKey,
        message: Self::Call,
        nonce: u64,
        flag: bool,
    ) -> Transaction<MockContext>;

    fn create_raw_txs(&self) -> Vec<RawTx> {
        let mut messages_iter = self.create_messages().into_iter().peekable();
        let mut serialized_messages = Vec::default();
        while let Some((sender, m, nonce)) = messages_iter.next() {
            let flag = messages_iter.peek().is_none();

            let tx = self.create_txs(sender, m, nonce, flag);

            serialized_messages.push(RawTx {
                data: tx.try_to_vec().unwrap(),
            })
        }
        serialized_messages
    }
}

mod election_messages {
    use super::*;

    pub struct ElectionCallMessages {}

    impl MessageGenerator for ElectionCallMessages {
        type Call = election::call::CallMessage<MockContext>;

        fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)> {
            let call_generator = &mut CallGenerator::new();
            let mut messages = Vec::default();

            messages.extend(call_generator.create_voters_and_vote());
            messages.extend(call_generator.freeze_vote());
            messages
        }

        fn create_txs(
            &self,
            sender: MockPublicKey,
            message: Self::Call,
            nonce: u64,
            _flag: bool,
        ) -> Transaction<MockContext> {
            Transaction::<MockContext>::new(
                Runtime::<MockContext>::encode_election_call(message),
                sender,
                MockSignature::default(),
                nonce,
            )
        }
    }

    pub struct InvalidElectionCallMessages {}

    impl MessageGenerator for InvalidElectionCallMessages {
        type Call = election::call::CallMessage<MockContext>;

        fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)> {
            let call_generator = &mut CallGenerator::new();
            let mut messages = Vec::default();

            messages.extend(call_generator.create_voters_and_vote());

            // Invalid message: This voter already voted.
            {
                let voter = MockPublicKey::try_from("voter_1").unwrap();
                let vote_message = election::call::CallMessage::Vote(1);
                messages.push((voter, vote_message, 1));
            }

            messages.extend(call_generator.freeze_vote());
            messages
        }

        fn create_txs(
            &self,
            sender: MockPublicKey,
            message: Self::Call,
            nonce: u64,
            _flag: bool,
        ) -> Transaction<MockContext> {
            Transaction::<MockContext>::new(
                Runtime::<MockContext>::encode_election_call(message),
                sender,
                MockSignature::default(),
                nonce,
            )
        }
    }

    pub struct BadSigElectionCallMessages {}

    impl MessageGenerator for BadSigElectionCallMessages {
        type Call = election::call::CallMessage<MockContext>;

        fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)> {
            let call_generator = &mut CallGenerator::new();
            let mut messages = Vec::default();

            messages.extend(call_generator.create_voters_and_vote());
            messages.extend(call_generator.freeze_vote());
            messages
        }

        fn create_txs(
            &self,
            sender: MockPublicKey,
            message: Self::Call,
            nonce: u64,
            flag: bool,
        ) -> Transaction<MockContext> {
            Transaction::<MockContext>::new(
                Runtime::<MockContext>::encode_election_call(message),
                sender,
                MockSignature {
                    msg_sig: Vec::default(),
                    should_fail: flag,
                },
                nonce,
            )
        }
    }

    pub struct BadNonceElectionCallMessages {}

    impl MessageGenerator for BadNonceElectionCallMessages {
        type Call = election::call::CallMessage<MockContext>;

        fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)> {
            let call_generator = &mut CallGenerator::new();
            let mut messages = Vec::default();

            messages.extend(call_generator.create_voters_and_vote());
            messages.extend(call_generator.freeze_vote());
            messages
        }

        fn create_txs(
            &self,
            sender: MockPublicKey,
            message: Self::Call,
            nonce: u64,
            flag: bool,
        ) -> Transaction<MockContext> {
            let nonce = if flag { nonce + 1 } else { nonce };

            Transaction::<MockContext>::new(
                Runtime::<MockContext>::encode_election_call(message),
                sender,
                MockSignature::default(),
                nonce,
            )
        }
    }

    pub struct BadSerializationElectionCallMessages {}

    impl MessageGenerator for BadSerializationElectionCallMessages {
        type Call = election::call::CallMessage<MockContext>;

        fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)> {
            let call_generator = &mut CallGenerator::new();
            let mut messages = Vec::default();

            messages.extend(call_generator.create_voters_and_vote());
            messages.extend(call_generator.freeze_vote());
            messages
        }

        fn create_txs(
            &self,
            sender: MockPublicKey,
            message: Self::Call,
            nonce: u64,
            flag: bool,
        ) -> Transaction<MockContext> {
            let call_data = if flag {
                vec![1, 2, 3]
            } else {
                Runtime::<MockContext>::encode_election_call(message)
            };

            Transaction::<MockContext>::new(call_data, sender, MockSignature::default(), nonce)
        }
    }
}

mod value_setter_messages {

    use super::*;

    pub struct CallGenerator {
        pub value_setter_admin: MockPublicKey,
    }

    impl CallGenerator {
        fn value_setter_call_messages(
            &self,
        ) -> Vec<(MockPublicKey, value_setter::call::CallMessage, u64)> {
            let mut value_setter_admin_nonce = 0;
            let mut messages = Vec::default();

            let new_value = 99;

            let set_value_msg_1 =
                value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue {
                    new_value,
                });

            let new_value = 33;
            let set_value_msg_2 =
                value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue {
                    new_value,
                });

            messages.push((
                self.value_setter_admin.clone(),
                set_value_msg_1,
                value_setter_admin_nonce,
            ));

            value_setter_admin_nonce += 1;
            messages.push((
                self.value_setter_admin.clone(),
                set_value_msg_2,
                value_setter_admin_nonce,
            ));

            messages
        }
    }
    pub struct ValueSetterMessages {
        pub call_generator: CallGenerator,
    }

    impl MessageGenerator for ValueSetterMessages {
        type Call = value_setter::call::CallMessage;

        fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)> {
            self.call_generator.value_setter_call_messages()
        }

        fn create_txs(
            &self,
            sender: MockPublicKey,
            message: Self::Call,
            nonce: u64,
            _flag: bool,
        ) -> Transaction<MockContext> {
            Transaction::<MockContext>::new(
                Runtime::<MockContext>::encode_value_setter_call(message),
                sender,
                MockSignature::default(),
                nonce,
            )
        }
    }
}

struct CallGenerator {
    election_admin_nonce: u64,
    election_admin: MockPublicKey,
}

impl CallGenerator {
    fn new() -> Self {
        Self {
            election_admin_nonce: 0,
            election_admin: MockPublicKey::try_from("election_admin").unwrap(),
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

    pub(crate) fn generate_query_check_balance() -> Vec<u8> {
        let query_message = sequencer::query::QueryMessage::GetSequencerAddressAndBalance;
        Runtime::<MockContext>::encode_sequencer_query(query_message)
    }
}
