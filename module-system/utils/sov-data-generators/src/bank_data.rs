use sov_bank::{get_token_address, Bank, CallMessage, Coins, TokenConfig};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Context};

use crate::generate_address;

type C = DefaultContext;

struct TransferData {
    sender_address: Address,
    receiver_address: Address,
    token_address: Address,
}

struct BankMessageGenerator {
    token_mint_txs: Vec<TokenConfig<C>>,
    transfer_txs: Vec<TransferData>,
}

pub(crate) fn mint_token_tx<'a>(
    token_name: String,
    initial_balance: u64,
    salt: u64,
    bank: Bank<DefaultContext>,
) -> (Address, CallMessage<C>) {
    let sender_address = generate_address("just_sender");
    let receiver_address = generate_address("just_receiver");

    let token_address = get_token_address::<C>(&token_name, sender_address.as_ref(), salt);

    assert_ne!(sender_address, receiver_address);

    let sender_context = C::new(sender_address.clone());

    (
        token_address,
        CallMessage::CreateToken {
            salt,
            token_name,
            initial_balance,
            minter_address: sender_address.clone(),
            authorized_minters: vec![sender_address.clone()],
        },
    )
}

pub(crate) fn transfer_token_tx(
    initial_balance: u64,
    sender_address: Address,
    receiver_address: Address,
    token_address: Address,
) -> CallMessage<C> {
    let transfer_amount = 15;

    CallMessage::Transfer {
        to: receiver_address.clone(),
        coins: Coins {
            amount: transfer_amount,
            token_address: token_address.clone(),
        },
    }
}
