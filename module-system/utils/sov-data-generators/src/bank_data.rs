use std::rc::Rc;

use sov_bank::{get_token_address, Bank, CallMessage, Coins};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Context, EncodeCall, Module, PrivateKey, Spec};

use crate::MessageGenerator;

pub struct TransferData<C: Context> {
    pub sender_pkey: Rc<DefaultPrivateKey>,
    pub receiver_address: <C as Spec>::Address,
    pub token_address: <C as Spec>::Address,
    pub transfer_amount: u64,
}

pub struct MintData<C: Context> {
    pub token_name: String,
    pub salt: u64,
    pub initial_balance: u64,
    pub minter_address: <C as Spec>::Address,
    pub minter_pkey: Rc<DefaultPrivateKey>,
    pub authorized_minters: Vec<<C as Spec>::Address>,
}

pub struct BankMessageGenerator<C: Context> {
    pub token_mint_txs: Vec<MintData<C>>,
    pub transfer_txs: Vec<TransferData<C>>,
}

impl Default for BankMessageGenerator<DefaultContext> {
    fn default() -> Self {
        let minter_address = generate_address::<DefaultContext>("just_sender");
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
                receiver_address: generate_address::<DefaultContext>("just_receiver"),
                token_address: get_token_address::<DefaultContext>(
                    &token_name,
                    minter_address.as_ref(),
                    salt,
                ),
            }]),
        }
    }
}

pub(crate) fn mint_token_tx<C: Context>(mint_data: &MintData<C>) -> CallMessage<C> {
    CallMessage::CreateToken {
        salt: mint_data.salt,
        token_name: mint_data.token_name.clone(),
        initial_balance: mint_data.initial_balance,
        minter_address: mint_data.minter_address.clone(),
        authorized_minters: mint_data.authorized_minters.clone(),
    }
}

pub(crate) fn transfer_token_tx<C: Context>(transfer_data: &TransferData<C>) -> CallMessage<C> {
    CallMessage::Transfer {
        to: transfer_data.receiver_address.clone(),
        coins: Coins {
            amount: transfer_data.transfer_amount,
            token_address: transfer_data.token_address.clone(),
        },
    }
}

impl<C: Context> MessageGenerator for BankMessageGenerator<C> {
    type Module = Bank<C>;

    fn create_messages(
        &self,
    ) -> Vec<(
        std::rc::Rc<sov_modules_api::default_signature::private_key::DefaultPrivateKey>,
        <Self::Module as Module>::CallMessage,
        u64,
    )> {
        let mut messages = Vec::<(
            std::rc::Rc<sov_modules_api::default_signature::private_key::DefaultPrivateKey>,
            <Self::Module as Module>::CallMessage,
            u64,
        )>::new();

        let mut nonce = 0;

        for mint_message in &self.token_mint_txs {
            messages.push((
                mint_message.minter_pkey.clone(),
                mint_token_tx::<C>(mint_message),
                nonce,
            ));
            nonce += 1;
        }

        for transfer_message in &self.transfer_txs {
            messages.push((
                transfer_message.sender_pkey.clone(),
                transfer_token_tx::<C>(transfer_message),
                nonce,
            ));
            nonce += 1;
        }

        messages
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &sov_modules_api::default_signature::private_key::DefaultPrivateKey,
        message: <Self::Module as Module>::CallMessage,
        nonce: u64,
        _is_last: bool,
    ) -> sov_modules_api::transaction::Transaction<DefaultContext> {
        let message = Encoder::encode_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}
