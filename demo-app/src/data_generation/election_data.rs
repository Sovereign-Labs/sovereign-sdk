use std::rc::Rc;

use sov_modules_api::Hasher;
use sov_modules_api::{default_context::DefaultContext, Spec};

use super::*;

struct CallGenerator {
    election_admin_nonce: u64,
}

impl CallGenerator {
    fn new() -> Self {
        Self {
            election_admin_nonce: 0,
        }
    }

    fn inc_nonce(&mut self) {
        self.election_admin_nonce += 1;
    }

    fn create_voters_and_vote(
        &mut self,
        election_admin: Rc<DefaultPrivateKey>,
        voters: &[Rc<DefaultPrivateKey>],
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        let set_candidates_message = election::call::CallMessage::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        messages.push((
            election_admin.clone(),
            set_candidates_message,
            self.election_admin_nonce,
        ));
        self.inc_nonce();

        for voter in voters {
            let add_voter_message =
                election::call::CallMessage::AddVoter(voter.pub_key().to_address());

            messages.push((
                election_admin.clone(),
                add_voter_message,
                self.election_admin_nonce,
            ));

            let vote_message = election::call::CallMessage::Vote(1);
            messages.push((voter.clone(), vote_message, 0));
            self.inc_nonce();
        }

        messages
    }

    fn freeze_vote(
        &mut self,
        election_admin: Rc<DefaultPrivateKey>,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        let freeze_message = election::call::CallMessage::FreezeElection;
        messages.push((election_admin, freeze_message, self.election_admin_nonce));
        self.inc_nonce();

        messages
    }

    fn all_messages(
        &mut self,
        election_admin: Rc<DefaultPrivateKey>,
        voters: &[Rc<DefaultPrivateKey>],
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        messages.extend(self.create_voters_and_vote(election_admin.clone(), voters));
        messages.extend(self.freeze_vote(election_admin));
        messages
    }
}

pub struct ElectionCallMessages {
    pub(crate) election_admin: Rc<DefaultPrivateKey>,
    pub(crate) voters: Vec<Rc<DefaultPrivateKey>>,
}

impl MessageGenerator for ElectionCallMessages {
    type Call = election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new();
        call_generator.all_messages(self.election_admin.clone(), &self.voters)
    }

    fn create_tx(
        &self,
        sender: &DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Runtime::<DefaultContext>::encode_election_call(message);
        let mut hasher = <DefaultContext as Spec>::Hasher::new();
        hasher.update(&message);
        hasher.update(&nonce.to_le_bytes());

        let msg_hash = hasher.finalize();
        let sig = sender.sign(msg_hash);

        Transaction::<DefaultContext>::new(message, sender.pub_key(), sig, nonce)
    }
}
/*
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
*/
