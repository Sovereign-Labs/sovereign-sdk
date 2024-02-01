use std::rc::Rc;

use sov_bank::{get_token_address, Bank, CallMessage, Coins};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Context, EncodeCall, Module, PrivateKey, PublicKey, Spec};

use crate::{Message, MessageGenerator};

pub struct TransferData<C: Context> {
    pub sender_pkey: Rc<C::PrivateKey>,
    pub receiver_address: <C as Spec>::Address,
    pub token_address: <C as Spec>::Address,
    pub transfer_amount: u64,
}

pub struct MintData<C: Context> {
    pub token_name: String,
    pub salt: u64,
    pub initial_balance: u64,
    pub minter_address: <C as Spec>::Address,
    pub minter_pkey: Rc<C::PrivateKey>,
    pub authorized_minters: Vec<<C as Spec>::Address>,
}

pub struct BankMessageGenerator<C: Context> {
    pub token_mint_txs: Vec<MintData<C>>,
    pub transfer_txs: Vec<TransferData<C>>,
}

const DEFAULT_TOKEN_NAME: &str = "Token1";
const DEFAULT_SALT: u64 = 10;
const DEFAULT_PVT_KEY: &str = "236e80cb222c4ed0431b093b3ac53e6aa7a2273fe1f4351cd354989a823432a27b758bf2e7670fafaf6bf0015ce0ff5aa802306fc7e3f45762853ffc37180fe6";
const DEFAULT_CHAIN_ID: u64 = 0;
const DEFAULT_GAS_TIP: u64 = 0;
const DEFAULT_GAS_LIMIT: u64 = 0;
const DEFAULT_INIT_BALANCE: u64 = 1000000;

pub fn get_default_token_address() -> <DefaultContext as Spec>::Address {
    let minter_key = DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap();
    let minter_address = minter_key.default_address();
    let salt = DEFAULT_SALT;
    let token_name = DEFAULT_TOKEN_NAME.to_owned();
    get_token_address::<DefaultContext>(&token_name, minter_address.as_ref(), salt)
}

pub fn get_default_private_key() -> DefaultPrivateKey {
    DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap()
}

impl Default for BankMessageGenerator<DefaultContext> {
    fn default() -> Self {
        let minter_key = DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap();
        let minter_address = minter_key.default_address();
        let salt = DEFAULT_SALT;
        let token_name = DEFAULT_TOKEN_NAME.to_owned();
        let mint_data = MintData {
            token_name: token_name.clone(),
            salt,
            initial_balance: 1000,
            minter_address,
            minter_pkey: Rc::new(minter_key.clone()),
            authorized_minters: Vec::from([minter_address]),
        };
        Self {
            token_mint_txs: Vec::from([mint_data]),
            transfer_txs: Vec::from([TransferData {
                sender_pkey: Rc::new(minter_key),
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

impl BankMessageGenerator<DefaultContext> {
    /// Gets the default sender address and private key.
    fn default_address_with_pkey() -> (<DefaultContext as Spec>::Address, DefaultPrivateKey) {
        let pkey = DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap();
        let address = pkey.default_address();
        (address, pkey)
    }

    /// Generates random transfers between the default sender and random receivers for default token parameters.
    pub fn default_generate_random_transfers(n: u64) -> Self {
        Self::generate_random_transfers(
            n,
            DEFAULT_TOKEN_NAME.to_owned(),
            DEFAULT_SALT,
            DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap(),
        )
    }

    /// Generates random transfers between the default sender and random receivers.
    pub fn generate_random_transfers(
        n: u64,
        token_name: String,
        salt: u64,
        sender_pk: DefaultPrivateKey,
    ) -> Self {
        let sa = sender_pk.default_address();
        let token_address =
            sov_bank::get_token_address::<DefaultContext>(token_name.as_str(), sa.as_ref(), salt);

        let mut transfer_txs = vec![];
        for _ in 1..(n + 1) {
            let priv_key = DefaultPrivateKey::generate();
            let address: <DefaultContext as Spec>::Address = priv_key.pub_key().to_address();

            transfer_txs.push(TransferData {
                sender_pkey: Rc::new(sender_pk.clone()),
                receiver_address: address,
                token_address,
                transfer_amount: 1,
            });
        }

        BankMessageGenerator {
            token_mint_txs: vec![],
            transfer_txs,
        }
    }

    /// Generates a create token transaction for default token parameters.
    pub fn default_generate_create_token() -> Self {
        let (minter_address, pk) = Self::default_address_with_pkey();
        Self::generate_create_token(
            DEFAULT_TOKEN_NAME.to_owned(),
            DEFAULT_SALT,
            pk.into(),
            vec![minter_address],
            DEFAULT_INIT_BALANCE,
        )
    }

    /// Generates a create token transaction.
    pub fn generate_create_token(
        token_name: String,
        salt: u64,
        minter_pkey: std::rc::Rc<DefaultPrivateKey>,
        authorized_minters: Vec<<DefaultContext as Spec>::Address>,
        initial_balance: u64,
    ) -> Self {
        Self {
            token_mint_txs: vec![MintData {
                token_name,
                salt,
                initial_balance,
                minter_address: minter_pkey.default_address(),
                minter_pkey,
                authorized_minters,
            }],
            transfer_txs: vec![],
        }
    }

    pub fn create_invalid_transfer() -> Self {
        let minter_key = DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap();
        let minter_address = minter_key.default_address();
        let salt = DEFAULT_SALT;
        let token_name = DEFAULT_TOKEN_NAME.to_owned();
        let mint_data = MintData {
            token_name: token_name.clone(),
            salt,
            initial_balance: 1000,
            minter_address,
            minter_pkey: Rc::new(minter_key),
            authorized_minters: Vec::from([minter_address]),
        };
        Self {
            token_mint_txs: Vec::from([mint_data]),
            transfer_txs: Vec::from([
                TransferData {
                    sender_pkey: Rc::new(DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap()),
                    transfer_amount: 15,
                    receiver_address: generate_address::<DefaultContext>("just_receiver"),
                    token_address: get_token_address::<DefaultContext>(
                        &token_name,
                        minter_address.as_ref(),
                        salt,
                    ),
                },
                TransferData {
                    sender_pkey: Rc::new(DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap()),
                    // invalid transfer because transfer_amount > minted supply
                    transfer_amount: 5000,
                    receiver_address: generate_address::<DefaultContext>("just_receiver"),
                    token_address: get_token_address::<DefaultContext>(
                        &token_name,
                        minter_address.as_ref(),
                        salt,
                    ),
                },
            ]),
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
    type Context = C;

    fn create_messages(&self) -> Vec<Message<Self::Context, Self::Module>> {
        let mut messages = Vec::<Message<C, Bank<C>>>::new();

        let mut nonce = 0;

        for mint_message in &self.token_mint_txs {
            messages.push(Message::new(
                mint_message.minter_pkey.clone(),
                mint_token_tx::<C>(mint_message),
                DEFAULT_CHAIN_ID,
                DEFAULT_GAS_TIP,
                DEFAULT_GAS_LIMIT,
                nonce,
            ));
            nonce += 1;
        }

        for transfer_message in &self.transfer_txs {
            messages.push(Message::new(
                transfer_message.sender_pkey.clone(),
                transfer_token_tx::<C>(transfer_message),
                DEFAULT_CHAIN_ID,
                DEFAULT_GAS_TIP,
                DEFAULT_GAS_LIMIT,
                nonce,
            ));
            nonce += 1;
        }

        messages
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &<Self::Context as Spec>::PrivateKey,
        message: <Self::Module as Module>::CallMessage,
        chain_id: u64,
        gas_tip: u64,
        gas_limit: u64,
        nonce: u64,
        _is_last: bool,
    ) -> sov_modules_api::transaction::Transaction<C> {
        let message = Encoder::encode_call(message);
        Transaction::<C>::new_signed_tx(sender, message, chain_id, gas_tip, gas_limit, nonce)
    }
}

pub struct BadSerializationBankCallMessages;

impl BadSerializationBankCallMessages {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for BadSerializationBankCallMessages {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageGenerator for BadSerializationBankCallMessages {
    type Module = Bank<DefaultContext>;
    type Context = DefaultContext;

    fn create_messages(&self) -> Vec<Message<Self::Context, Self::Module>> {
        let mut messages = Vec::<Message<DefaultContext, Bank<DefaultContext>>>::new();
        let minter_key = DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap();
        let minter_address = minter_key.default_address();
        let salt = DEFAULT_SALT;
        let token_name = DEFAULT_TOKEN_NAME.to_owned();
        messages.push(Message::new(
            Rc::new(DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap()),
            CallMessage::CreateToken {
                salt,
                token_name,
                initial_balance: 1000,
                minter_address,
                authorized_minters: Vec::from([minter_address]),
            },
            DEFAULT_CHAIN_ID,
            DEFAULT_GAS_TIP,
            DEFAULT_GAS_LIMIT,
            0,
        ));
        messages.push(Message::new(
            Rc::new(DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap()),
            CallMessage::Transfer {
                to: generate_address::<DefaultContext>("just_receiver"),
                coins: Coins {
                    amount: 50,
                    token_address: get_default_token_address(),
                },
            },
            DEFAULT_CHAIN_ID,
            DEFAULT_GAS_TIP,
            DEFAULT_GAS_LIMIT,
            0,
        ));
        messages
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Bank<DefaultContext> as Module>::CallMessage,
        chain_id: u64,
        gas_tip: u64,
        gas_limit: u64,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext> {
        // just some random bytes that won't deserialize to a valid txn
        let call_data = if is_last {
            vec![1, 2, 3]
        } else {
            Encoder::encode_call(message)
        };

        Transaction::<DefaultContext>::new_signed_tx(
            sender, call_data, chain_id, gas_tip, gas_limit, nonce,
        )
    }
}

pub struct BadSignatureBankCallMessages;

impl BadSignatureBankCallMessages {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for BadSignatureBankCallMessages {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageGenerator for BadSignatureBankCallMessages {
    type Module = Bank<DefaultContext>;
    type Context = DefaultContext;

    fn create_messages(&self) -> Vec<Message<Self::Context, Self::Module>> {
        let mut messages = Vec::<Message<DefaultContext, Bank<DefaultContext>>>::new();
        let minter_key = DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap();
        let minter_address = minter_key.default_address();
        let salt = DEFAULT_SALT;
        let token_name = DEFAULT_TOKEN_NAME.to_owned();
        messages.push(Message::new(
            Rc::new(DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap()),
            CallMessage::CreateToken {
                salt,
                token_name,
                initial_balance: 1000,
                minter_address,
                authorized_minters: Vec::from([minter_address]),
            },
            DEFAULT_CHAIN_ID,
            DEFAULT_GAS_TIP,
            DEFAULT_GAS_LIMIT,
            0,
        ));
        messages
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Bank<DefaultContext> as Module>::CallMessage,
        chain_id: u64,
        gas_tip: u64,
        gas_limit: u64,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext> {
        let call_data = Encoder::encode_call(message);

        if is_last {
            let tx = Transaction::<DefaultContext>::new_signed_tx(
                sender,
                call_data.clone(),
                chain_id,
                gas_tip,
                gas_limit,
                nonce,
            );
            Transaction::new(
                DefaultPrivateKey::generate().pub_key(),
                call_data,
                tx.signature().clone(),
                chain_id,
                gas_tip,
                gas_limit,
                nonce,
            )
        } else {
            Transaction::<DefaultContext>::new_signed_tx(
                sender, call_data, chain_id, gas_tip, gas_limit, nonce,
            )
        }
    }
}

pub struct BadNonceBankCallMessages;

impl BadNonceBankCallMessages {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for BadNonceBankCallMessages {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageGenerator for BadNonceBankCallMessages {
    type Module = Bank<DefaultContext>;
    type Context = DefaultContext;

    fn create_messages(&self) -> Vec<Message<Self::Context, Self::Module>> {
        let mut messages = Vec::<Message<DefaultContext, Bank<DefaultContext>>>::new();
        let minter_key = DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap();
        let minter_address = minter_key.default_address();
        let salt = DEFAULT_SALT;
        let token_name = DEFAULT_TOKEN_NAME.to_owned();
        messages.push(Message::new(
            Rc::new(DefaultPrivateKey::from_hex(DEFAULT_PVT_KEY).unwrap()),
            CallMessage::CreateToken {
                salt,
                token_name,
                initial_balance: 1000,
                minter_address,
                authorized_minters: Vec::from([minter_address]),
            },
            DEFAULT_CHAIN_ID,
            DEFAULT_GAS_TIP,
            DEFAULT_GAS_LIMIT,
            0,
        ));
        messages
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Bank<DefaultContext> as Module>::CallMessage,
        chain_id: u64,
        gas_tip: u64,
        gas_limit: u64,
        _nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Encoder::encode_call(message);
        // hard-coding the nonce to 1000
        Transaction::<DefaultContext>::new_signed_tx(
            sender, message, chain_id, gas_tip, gas_limit, 1000,
        )
    }
}
