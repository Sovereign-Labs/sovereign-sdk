use std::rc::Rc;

use sov_bank::{get_token_address, Bank, CallMessage, Coins};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Address, Module, Spec};

use crate::{generate_address, MessageGenerator, Runtime};

type C = DefaultContext;

pub struct TransferData {
    pub sender_pkey: Rc<DefaultPrivateKey>,
    pub receiver_address: Address,
    pub token_address: Address,
    pub transfer_amount: u64,
}

pub struct MintData {
    pub token_name: String,
    pub salt: u64,
    pub initial_balance: u64,
    pub minter_address: <DefaultContext as Spec>::Address,
    pub minter_pkey: Rc<DefaultPrivateKey>,
    pub authorized_minters: Vec<<DefaultContext as Spec>::Address>,
}

pub struct BankMessageGenerator {
    pub token_mint_txs: Vec<MintData>,
    pub transfer_txs: Vec<TransferData>,
}

impl Default for BankMessageGenerator {
    fn default() -> Self {
        let minter_address = generate_address("just_sender");
        let salt = 10;
        let token_name = "Token1".to_owned();
        let mint_data = MintData {
            token_name: token_name.clone(),
            salt,
            initial_balance: 1000,
            minter_address,
            minter_pkey: Rc::new(DefaultPrivateKey::generate()),
            authorized_minters: Vec::from([minter_address]),
        };
        Self {
            token_mint_txs: Vec::from([mint_data]),
            transfer_txs: Vec::from([TransferData {
                sender_pkey: Rc::new(DefaultPrivateKey::generate()),
                transfer_amount: 15,
                receiver_address: generate_address("just_receiver"),
                token_address: get_token_address::<C>(&token_name, minter_address.as_ref(), salt),
            }]),
        }
    }
}

pub(crate) fn mint_token_tx(mint_data: &MintData) -> CallMessage<C> {
    CallMessage::CreateToken {
        salt: mint_data.salt,
        token_name: mint_data.token_name.clone(),
        initial_balance: mint_data.initial_balance,
        minter_address: mint_data.minter_address.clone(),
        authorized_minters: mint_data.authorized_minters.clone(),
    }
}

pub(crate) fn transfer_token_tx(transfer_data: &TransferData) -> CallMessage<C> {
    CallMessage::Transfer {
        to: transfer_data.receiver_address,
        coins: Coins {
            amount: transfer_data.transfer_amount,
            token_address: transfer_data.token_address.clone(),
        },
    }
}

impl MessageGenerator for BankMessageGenerator {
    type Call = <Bank<DefaultContext> as Module>::CallMessage;

    fn create_messages(
        &self,
    ) -> Vec<(
        std::rc::Rc<sov_modules_api::default_signature::private_key::DefaultPrivateKey>,
        Self::Call,
        u64,
    )> {
        let mut messages = Vec::<(
            std::rc::Rc<sov_modules_api::default_signature::private_key::DefaultPrivateKey>,
            Self::Call,
            u64,
        )>::new();

        let mut nonce = 0;

        for mint_message in &self.token_mint_txs {
            messages.push((
                mint_message.minter_pkey.clone(),
                mint_token_tx(mint_message),
                nonce,
            ));
            nonce += 1;
        }

        for transfer_message in &self.transfer_txs {
            messages.push((
                transfer_message.sender_pkey.clone(),
                transfer_token_tx(transfer_message),
                nonce,
            ));
            nonce += 1;
        }

        messages
    }

    fn create_tx(
        &self,
        sender: &sov_modules_api::default_signature::private_key::DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        _is_last: bool,
    ) -> sov_modules_api::transaction::Transaction<DefaultContext> {
        let message = Runtime::encode_bank_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}
