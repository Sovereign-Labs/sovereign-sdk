use std::marker::PhantomData;
use std::rc::Rc;

use sov_election::Election;
use sov_modules_api::{EncodeCall, PrivateKey, PublicKey};

use super::*;

struct CallGenerator<C: Context> {
    election_admin_nonce: u64,
    election_admin: Rc<C::PrivateKey>,
    voters: Vec<Rc<C::PrivateKey>>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> CallGenerator<C> {
    fn new(election_admin: Rc<C::PrivateKey>) -> Self {
        let voters = vec![
            Rc::new(C::PrivateKey::generate()),
            Rc::new(C::PrivateKey::generate()),
            Rc::new(C::PrivateKey::generate()),
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

    fn create_voters_and_vote(&mut self) -> Vec<Message<C, Election<C>>> {
        let mut messages = Vec::default();

        let set_candidates_message = sov_election::CallMessage::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        messages.push(Message::new(
            self.election_admin.clone(),
            set_candidates_message,
            self.election_admin_nonce,
        ));
        self.inc_nonce();

        for voter in self.voters.clone() {
            let add_voter_message =
                sov_election::CallMessage::AddVoter(voter.pub_key().to_address());

            messages.push(Message::new(
                self.election_admin.clone(),
                add_voter_message,
                self.election_admin_nonce,
            ));

            let vote_message = sov_election::CallMessage::Vote(1);
            messages.push(Message::new(voter, vote_message, 0));
            self.inc_nonce();
        }

        messages
    }

    fn freeze_vote(&mut self) -> Vec<Message<C, Election<C>>> {
        let mut messages = Vec::default();

        let freeze_message = sov_election::CallMessage::FreezeElection;
        messages.push(Message::new(
            self.election_admin.clone(),
            freeze_message,
            self.election_admin_nonce,
        ));
        self.inc_nonce();

        messages
    }

    fn all_messages(&mut self) -> Vec<Message<C, Election<C>>> {
        let mut messages = Vec::default();

        messages.extend(self.create_voters_and_vote());
        messages.extend(self.freeze_vote());
        messages
    }
}

pub struct ElectionCallMessages<C: Context> {
    election_admin: Rc<C::PrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> ElectionCallMessages<C> {
    pub fn new(election_admin: C::PrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for ElectionCallMessages<C> {
    type Module = Election<C>;
    type Context = C;

    fn create_messages(&self) -> Vec<Message<C, Election<C>>> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &C::PrivateKey,
        message: <Self::Module as Module>::CallMessage,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<C> {
        let message = Encoder::encode_call(message);
        Transaction::<C>::new_signed_tx(sender, message, nonce)
    }
}

pub struct InvalidElectionCallMessages<C: Context> {
    election_admin: Rc<C::PrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> InvalidElectionCallMessages<C> {
    pub fn new(election_admin: C::PrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for InvalidElectionCallMessages<C> {
    type Module = Election<C>;
    type Context = C;

    fn create_messages(&self) -> Vec<Message<C, Election<C>>> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());

        let mut messages = Vec::default();

        messages.extend(call_generator.create_voters_and_vote());

        // Additional invalid message: This voter already voted.
        {
            // Need to do the cloning in two steps because type inference doesn't work otherwise
            let voter_ref: &Rc<<C as Spec>::PrivateKey> = &call_generator.voters[0];
            let voter = voter_ref.clone();
            let vote_message = sov_election::CallMessage::Vote(1);
            messages.push(Message::new(voter, vote_message, 1));
        }

        messages.extend(call_generator.freeze_vote());
        messages
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &C::PrivateKey,
        message: <Election<C> as Module>::CallMessage,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<C> {
        let message = Encoder::encode_call(message);
        Transaction::<C>::new_signed_tx(sender, message, nonce)
    }
}

pub struct BadSigElectionCallMessages<C: Context> {
    election_admin: Rc<C::PrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> BadSigElectionCallMessages<C> {
    pub fn new(election_admin: C::PrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for BadSigElectionCallMessages<C> {
    type Module = Election<C>;
    type Context = C;

    fn create_messages(&self) -> Vec<Message<C, Election<C>>> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &C::PrivateKey,
        message: <Election<C> as Module>::CallMessage,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<C> {
        let message = Encoder::encode_call(message);

        if is_last {
            let tx = Transaction::<C>::new_signed_tx(sender, message.clone(), nonce);
            Transaction::new(
                C::PrivateKey::generate().pub_key(),
                message,
                tx.signature().clone(),
                nonce,
            )
        } else {
            Transaction::<C>::new_signed_tx(sender, message, nonce)
        }
    }
}

pub struct BadNonceElectionCallMessages<C: Context> {
    election_admin: Rc<C::PrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> BadNonceElectionCallMessages<C> {
    pub fn new(election_admin: C::PrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for BadNonceElectionCallMessages<C> {
    type Module = Election<C>;
    type Context = C;

    fn create_messages(&self) -> Vec<Message<C, Election<C>>> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &C::PrivateKey,
        message: <Election<C> as Module>::CallMessage,
        nonce: u64,
        flag: bool,
    ) -> Transaction<C> {
        let nonce = if flag { nonce + 1 } else { nonce };

        let message = Encoder::encode_call(message);
        Transaction::<C>::new_signed_tx(sender, message, nonce)
    }
}

pub struct BadSerializationElectionCallMessages<C: Context> {
    election_admin: Rc<C::PrivateKey>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> BadSerializationElectionCallMessages<C> {
    pub fn new(election_admin: C::PrivateKey) -> Self {
        Self {
            election_admin: Rc::new(election_admin),
            phantom_context: Default::default(),
        }
    }
}

impl<C: Context> MessageGenerator for BadSerializationElectionCallMessages<C> {
    type Module = Election<C>;
    type Context = C;

    fn create_messages(&self) -> Vec<Message<C, Election<C>>> {
        let call_generator = &mut CallGenerator::new(self.election_admin.clone());
        call_generator.all_messages()
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &C::PrivateKey,
        message: <Election<C> as Module>::CallMessage,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<C> {
        let call_data = if is_last {
            vec![1, 2, 3]
        } else {
            Encoder::encode_call(message)
        };

        Transaction::<C>::new_signed_tx(sender, call_data, nonce)
    }
}
