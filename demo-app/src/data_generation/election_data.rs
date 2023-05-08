use sov_modules_api::context::DefaultContext;

use super::*;

struct CallGenerator {
    election_admin_nonce: u64,
    election_admin: DefaultPublicKey,
}

impl CallGenerator {
    fn new() -> Self {
        Self {
            election_admin_nonce: 0,
            election_admin: DefaultPublicKey::from("election_admin"),
        }
    }

    fn inc_nonce(&mut self) {
        self.election_admin_nonce += 1;
    }

    fn create_voters_and_vote(
        &mut self,
    ) -> Vec<(
        DefaultPublicKey,
        election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        let set_candidates_message = election::call::CallMessage::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        messages.push((
            self.election_admin.clone(),
            set_candidates_message,
            self.election_admin_nonce,
        ));
        self.inc_nonce();

        let voters = vec![
            DefaultPublicKey::from("voter_1"),
            DefaultPublicKey::from("voter_2"),
            DefaultPublicKey::from("voter_3"),
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
            self.inc_nonce();
        }

        messages
    }

    fn freeze_vote(
        &mut self,
    ) -> Vec<(
        DefaultPublicKey,
        election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        let freeze_message = election::call::CallMessage::FreezeElection;
        messages.push((
            self.election_admin.clone(),
            freeze_message,
            self.election_admin_nonce,
        ));
        self.inc_nonce();

        messages
    }

    fn all_messages(
        &mut self,
    ) -> Vec<(
        DefaultPublicKey,
        election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        messages.extend(self.create_voters_and_vote());
        messages.extend(self.freeze_vote());
        messages
    }
}

pub struct ElectionCallMessages {}

impl MessageGenerator for ElectionCallMessages {
    type Call = election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(DefaultPublicKey, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new();
        call_generator.all_messages()
    }

    fn create_tx(
        &self,
        sender: DefaultPublicKey,
        message: Self::Call,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        Transaction::<DefaultContext>::new(
            Runtime::<DefaultContext>::encode_election_call(message),
            sender,
            DefaultSignature::default(),
            nonce,
        )
    }
}

pub struct InvalidElectionCallMessages {}

impl MessageGenerator for InvalidElectionCallMessages {
    type Call = election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(DefaultPublicKey, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new();
        let mut messages = Vec::default();

        messages.extend(call_generator.create_voters_and_vote());

        // Invalid message: This voter already voted.
        {
            let voter = DefaultPublicKey::from("voter_1");
            let vote_message = election::call::CallMessage::Vote(1);
            messages.push((voter, vote_message, 1));
        }

        messages.extend(call_generator.freeze_vote());
        messages
    }

    fn create_tx(
        &self,
        sender: DefaultPublicKey,
        message: Self::Call,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        Transaction::<DefaultContext>::new(
            Runtime::<DefaultContext>::encode_election_call(message),
            sender,
            DefaultSignature::default(),
            nonce,
        )
    }
}

pub struct BadSigElectionCallMessages {}

impl MessageGenerator for BadSigElectionCallMessages {
    type Call = election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(DefaultPublicKey, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new();
        call_generator.all_messages()
    }

    fn create_tx(
        &self,
        sender: DefaultPublicKey,
        message: Self::Call,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext> {
        Transaction::<DefaultContext>::new(
            Runtime::<DefaultContext>::encode_election_call(message),
            sender,
            DefaultSignature {
                msg_sig: Vec::default(),
                should_fail: is_last,
            },
            nonce,
        )
    }
}

pub struct BadNonceElectionCallMessages {}

impl MessageGenerator for BadNonceElectionCallMessages {
    type Call = election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(DefaultPublicKey, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new();
        call_generator.all_messages()
    }

    fn create_tx(
        &self,
        sender: DefaultPublicKey,
        message: Self::Call,
        nonce: u64,
        flag: bool,
    ) -> Transaction<DefaultContext> {
        let nonce = if flag { nonce + 1 } else { nonce };

        Transaction::<DefaultContext>::new(
            Runtime::<DefaultContext>::encode_election_call(message),
            sender,
            DefaultSignature::default(),
            nonce,
        )
    }
}

pub struct BadSerializationElectionCallMessages {}

impl MessageGenerator for BadSerializationElectionCallMessages {
    type Call = election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(DefaultPublicKey, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new();
        call_generator.all_messages()
    }

    fn create_tx(
        &self,
        sender: DefaultPublicKey,
        message: Self::Call,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext> {
        let call_data = if is_last {
            vec![1, 2, 3]
        } else {
            Runtime::<DefaultContext>::encode_election_call(message)
        };

        Transaction::<DefaultContext>::new(call_data, sender, DefaultSignature::default(), nonce)
    }
}
