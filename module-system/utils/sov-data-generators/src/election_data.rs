use std::marker::PhantomData;
use std::rc::Rc;

use sov_election::Election;
use sov_modules_api::{EncodeCall, PublicKey};

use super::*;

struct CallGenerator<C: Context> {
    election_admin_nonce: u64,
    election_admin: Rc<DefaultPrivateKey>,
    voters: Vec<Rc<DefaultPrivateKey>>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> CallGenerator<C> {
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
            phantom_context: Default::default(),
        }
    }

    fn inc_nonce(&mut self) {
        self.election_admin_nonce += 1;
    }

    fn create_voters_and_vote(
        &mut self,
    ) -> Vec<(Rc<DefaultPrivateKey>, sov_election::CallMessage<C>, u64)> {
        let mut messages = Vec::default();

        let set_candidates_message = sov_election::CallMessage::SetCandidates {
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
                sov_election::CallMessage::AddVoter(voter.pub_key().to_address());

            messages.push((
                self.election_admin.clone(),
                add_voter_message,
                self.election_admin_nonce,
            ));

            let vote_message = sov_election::CallMessage::Vote(1);
            messages.push((voter, vote_message, 0));
            self.inc_nonce();
        }

        messages
    }

    fn freeze_vote(&mut self) -> Vec<(Rc<DefaultPrivateKey>, sov_election::CallMessage<C>, u64)> {
        let mut messages = Vec::default();

        let freeze_message = sov_election::CallMessage::FreezeElection;
        messages.push((
            self.election_admin.clone(),
            freeze_message,
            self.election_admin_nonce,
        ));
        self.inc_nonce();

        messages
    }

    fn all_messages(&mut self) -> Vec<(Rc<DefaultPrivateKey>, sov_election::CallMessage<C>, u64)> {
        let mut messages = Vec::default();

        messages.extend(self.create_voters_and_vote());
        messages.extend(self.freeze_vote());
        messages
    }
}

pub struct ElectionCallMessages<C: Context> {
    election_admin: Rc<DefaultPrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> ElectionCallMessages<C> {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for ElectionCallMessages<C> {
    type Module = Election<C>;

    fn create_messages(
        &self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        <Self::Module as Module>::CallMessage,
        u64,
    )> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Self::Module as Module>::CallMessage,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Encoder::encode_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}

pub struct InvalidElectionCallMessages<C: Context> {
    election_admin: Rc<DefaultPrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> InvalidElectionCallMessages<C> {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for InvalidElectionCallMessages<C> {
    type Module = Election<C>;

    fn create_messages(
        &self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        <Election<C> as Module>::CallMessage,
        u64,
    )> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());

        let mut messages = Vec::default();

        messages.extend(call_generator.create_voters_and_vote());

        // Additional invalid message: This voter already voted.
        {
            let voter = call_generator.voters[0].clone();
            let vote_message = sov_election::CallMessage::Vote(1);
            messages.push((voter, vote_message, 1));
        }

        messages.extend(call_generator.freeze_vote());
        messages
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Election<C> as Module>::CallMessage,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Encoder::encode_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}

pub struct BadSigElectionCallMessages<C: Context> {
    election_admin: Rc<DefaultPrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> BadSigElectionCallMessages<C> {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for BadSigElectionCallMessages<C> {
    type Module = Election<C>;

    fn create_messages(
        &self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        <Election<C> as Module>::CallMessage,
        u64,
    )> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Election<C> as Module>::CallMessage,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Encoder::encode_call(message);

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

pub struct BadNonceElectionCallMessages<C: Context> {
    election_admin: Rc<DefaultPrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> BadNonceElectionCallMessages<C> {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for BadNonceElectionCallMessages<C> {
    type Module = Election<C>;

    fn create_messages(
        &self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        <Election<C> as Module>::CallMessage,
        u64,
    )> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Election<C> as Module>::CallMessage,
        nonce: u64,
        flag: bool,
    ) -> Transaction<DefaultContext> {
        let nonce = if flag { nonce + 1 } else { nonce };

        let message = Encoder::encode_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}

pub struct BadSerializationElectionCallMessages<C: Context> {
    election_admin: Rc<DefaultPrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> BadSerializationElectionCallMessages<C> {
    pub fn new(election_admin: DefaultPrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for BadSerializationElectionCallMessages<C> {
    type Module = Election<C>;

    fn create_messages(
        &self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        <Election<C> as Module>::CallMessage,
        u64,
    )> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Election<C> as Module>::CallMessage,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext> {
        let call_data = if is_last {
            vec![1, 2, 3]
        } else {
            Encoder::encode_call(message)
        };

        Transaction::<DefaultContext>::new_signed_tx(sender, call_data, nonce)
    }
}
