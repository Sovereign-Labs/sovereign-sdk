use ibc::{applications::transfer::context::{TokenTransferValidationContext, TokenTransferExecutionContext}, Signer};

use crate::Transfer;


impl<C> TokenTransferValidationContext for Transfer<C> where C: sov_modules_api::Context {
    type AccountId = Address<C>;

    fn get_port(&self) -> Result<ibc::core::ics24_host::identifier::PortId, ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }

    fn get_escrow_account(
        &self,
        port_id: &ibc::core::ics24_host::identifier::PortId,
        channel_id: &ibc::core::ics24_host::identifier::ChannelId,
    ) -> Result<Self::AccountId, ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }

    fn can_send_coins(&self) -> Result<(), ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }

    fn can_receive_coins(&self) -> Result<(), ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }

    fn send_coins_validate(
        &self,
        from_account: &Self::AccountId,
        to_account: &Self::AccountId,
        coin: &ibc::applications::transfer::PrefixedCoin,
    ) -> Result<(), ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }

    fn mint_coins_validate(
        &self,
        account: &Self::AccountId,
        coin: &ibc::applications::transfer::PrefixedCoin,
    ) -> Result<(), ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }

    fn burn_coins_validate(
        &self,
        account: &Self::AccountId,
        coin: &ibc::applications::transfer::PrefixedCoin,
    ) -> Result<(), ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }
}

impl<C> TokenTransferExecutionContext for Transfer<C> where C: sov_modules_api::Context {
    fn send_coins_execute(
        &mut self,
        from_account: &Self::AccountId,
        to_account: &Self::AccountId,
        coin: &ibc::applications::transfer::PrefixedCoin,
    ) -> Result<(), ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }

    fn mint_coins_execute(
        &mut self,
        account: &Self::AccountId,
        coin: &ibc::applications::transfer::PrefixedCoin,
    ) -> Result<(), ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }

    fn burn_coins_execute(
        &mut self,
        account: &Self::AccountId,
        coin: &ibc::applications::transfer::PrefixedCoin,
    ) -> Result<(), ibc::applications::transfer::error::TokenTransferError> {
        todo!()
    }
}


pub struct Address<C: sov_modules_api::Context> {
    address: C::Address
}

impl<C> From<Signer> for Address<C> where C: sov_modules_api::Context {
    fn from(signer: Signer) -> Self {
        todo!()
    }
}
