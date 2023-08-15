use ibc::applications::transfer::context::{
    TokenTransferExecutionContext, TokenTransferValidationContext,
};
use ibc::applications::transfer::error::TokenTransferError;
use ibc::applications::transfer::{PrefixedCoin, PORT_ID_STR};
use ibc::core::ics24_host::identifier::{ChannelId, PortId};
use ibc::Signer;

use crate::Transfer;

impl<C> TokenTransferValidationContext for Transfer<C>
where
    C: sov_modules_api::Context,
{
    type AccountId = Address<C>;

    fn get_port(&self) -> Result<PortId, TokenTransferError> {
        PortId::new(PORT_ID_STR.to_string()).map_err(TokenTransferError::InvalidIdentifier)
    }

    fn get_escrow_account(
        &self,
        port_id: &PortId,
        channel_id: &ChannelId,
    ) -> Result<Self::AccountId, TokenTransferError> {
        todo!()
    }

    fn can_send_coins(&self) -> Result<(), TokenTransferError> {
        Ok(())
    }

    fn can_receive_coins(&self) -> Result<(), TokenTransferError> {
        Ok(())
    }

    fn send_coins_validate(
        &self,
        from_account: &Self::AccountId,
        to_account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }

    fn mint_coins_validate(
        &self,
        account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }

    fn burn_coins_validate(
        &self,
        account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }
}

impl<C> TokenTransferExecutionContext for Transfer<C>
where
    C: sov_modules_api::Context,
{
    fn send_coins_execute(
        &mut self,
        from_account: &Self::AccountId,
        to_account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }

    fn mint_coins_execute(
        &mut self,
        account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }

    fn burn_coins_execute(
        &mut self,
        account: &Self::AccountId,
        coin: &PrefixedCoin,
    ) -> Result<(), TokenTransferError> {
        todo!()
    }
}

/// Address type, which wraps C::Address. This is needed to implement
/// `TryFrom<Signer>` (circumventing the orphan rule).
pub struct Address<C: sov_modules_api::Context> {
    address: C::Address,
}

impl<C> TryFrom<Signer> for Address<C>
where
    C: sov_modules_api::Context,
{
    type Error = anyhow::Error;

    fn try_from(signer: Signer) -> Result<Self, Self::Error> {
        Ok(Address {
            address: signer.as_ref().parse()?,
        })
    }
}
