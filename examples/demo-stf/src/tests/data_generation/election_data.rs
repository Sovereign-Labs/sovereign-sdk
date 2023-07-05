use std::rc::Rc;

use sov_modules_api::default_context::DefaultContext;

use super::*;

struct CallGenerator {
    election_admin_nonce: u64,
    election_admin: Rc<DefaultPrivateKey>,
    voters: Vec<Rc<DefaultPrivateKey>>,
}

impl CallGenerator {
    fn new(election_admin: Rc<DefaultPrivateKey>) -> Self {
        let voters = vec![
            Rc::new(DefaultPrivateKey::generate()),
            Rc::new(DefaultPrivateKey::generate()),
            Rc::new(DefaultPrivateKey::generate()),
        ];
        Self {
            election_admin_nonce: 0,
            election_admin,
            voters,
        }
    }

    fn inc_nonce(&mut self) {
        self.election_admin_nonce += 1;
    }

    fn create_voters_and_vote(
        &mut self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        sov_election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        let set_candidates_message = sov_election::call::CallMessage::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        messages.push((
            self.election_admin.clone(),
            set_candidates_message,
            self.election_admin_nonce,
        ));
        self.inc_nonce();

        for voter in self.voters.clone() {
            let add_voter_message =
                sov_election::call::CallMessage::AddVoter(voter.pub_key().to_address());

            messages.push((
                self.election_admin.clone(),
                add_voter_message,
                self.election_admin_nonce,
            ));

            let vote_message = sov_election::call::CallMessage::Vote(1);
            messages.push((voter, vote_message, 0));
            self.inc_nonce();
        }

        messages
    }

    fn freeze_vote(
        &mut self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        sov_election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        let freeze_message = sov_election::call::CallMessage::FreezeElection;
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
        Rc<DefaultPrivateKey>,
        sov_election::call::CallMessage<DefaultContext>,
        u64,
    )> {
        let mut messages = Vec::default();

        messages.extend(self.create_voters_and_vote());
        messages.extend(self.freeze_vote());
        messages
    }
}

pub struct ElectionCallMessages {
    election_admin: Rc<DefaultPrivateKey>,
}

impl ElectionCallMessages {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
        }
    }
}

impl MessageGenerator for ElectionCallMessages {
    type Call = sov_election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx(
        &self,
        sender: &DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Runtime::<DefaultContext>::encode_election_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}

pub struct InvalidElectionCallMessages {
    election_admin: Rc<DefaultPrivateKey>,
}

impl InvalidElectionCallMessages {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
        }
    }
}

impl MessageGenerator for InvalidElectionCallMessages {
    type Call = sov_election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());

        let mut messages = Vec::default();

        messages.extend(call_generator.create_voters_and_vote());

        // Additional invalid message: This voter already voted.
        {
            let voter = call_generator.voters[0].clone();
            let vote_message = sov_election::call::CallMessage::Vote(1);
            messages.push((voter, vote_message, 1));
        }

        messages.extend(call_generator.freeze_vote());
        messages
    }

    fn create_tx(
        &self,
        sender: &DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Runtime::<DefaultContext>::encode_election_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}

pub struct BadSigElectionCallMessages {
    election_admin: Rc<DefaultPrivateKey>,
}

impl BadSigElectionCallMessages {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
        }
    }
}

impl MessageGenerator for BadSigElectionCallMessages {
    type Call = sov_election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx(
        &self,
        sender: &DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Runtime::<DefaultContext>::encode_election_call(message);

        if is_last {
            let tx = Transaction::<DefaultContext>::new_signed_tx(sender, message.clone(), nonce);
            Transaction::new(
                DefaultPrivateKey::generate().pub_key(),
                message,
                tx.signature().clone(),
                nonce,
            )
        } else {
            Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
        }
    }
}

pub struct BadNonceElectionCallMessages {
    election_admin: Rc<DefaultPrivateKey>,
}

impl BadNonceElectionCallMessages {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
        }
    }
}

impl MessageGenerator for BadNonceElectionCallMessages {
    type Call = sov_election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx(
        &self,
        sender: &DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        flag: bool,
    ) -> Transaction<DefaultContext> {
        let nonce = if flag { nonce + 1 } else { nonce };

        let message = Runtime::<DefaultContext>::encode_election_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}

pub struct BadSerializationElectionCallMessages {
    election_admin: Rc<DefaultPrivateKey>,
}

impl BadSerializationElectionCallMessages {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
        }
    }
}

impl MessageGenerator for BadSerializationElectionCallMessages {
    type Call = sov_election::call::CallMessage<DefaultContext>;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx(
        &self,
        sender: &DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext> {
        let call_data = if is_last {
            vec![1, 2, 3]
        } else {
            Runtime::<DefaultContext>::encode_election_call(message)
        };

        Transaction::<DefaultContext>::new_signed_tx(sender, call_data, nonce)
    }
}
