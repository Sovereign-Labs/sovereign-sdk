use std::str::FromStr as _;

use sov_modules_api::macros::serialize;
use sov_modules_api::{
    err_detail, Base58Address, Context, CoreModuleError, CredentialId, ErrorContext, ErrorDetail,
    HexHash, HexString, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, TxState, EventEmitter,
};

use sov_hyperlane_integration::{HyperlaneAddress, Ism, Recipient, Warp};

#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct SolanaRegistration<S: Spec>
where
    S::Address: HyperlaneAddress,
{
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    /// The inner module that we will fall back to if the origin domain is not the configured
    /// solana domain or the sender is not the configured solana program id.
    #[module]
    warp: Warp<S>,

    #[module]
    accounts: sov_accounts::Accounts<S>,
}

#[derive(thiserror::Error, Debug, serde::Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum SolanaRegistrationError {
    #[error("Core module error: {0}")]
    CoreModuleError(#[from] CoreModuleError),
    #[error("Module doesn't support calls")]
    UnsupportedModuleCall,
    #[error("Embedded pubkey already registered to different address. Attempted: {attempted_address}, Registered: {registered_address}")]
    AlreadyRegistered {
        attempted_address: String,
        registered_address: String,
    },
    #[error("Invalid body length. Expected {expected}, found {found}")]
    InvalidBodyLength { expected: usize, found: usize },
    #[error("Failed to extract public key from body")]
    ExtractPubKey,
}

impl ErrorDetail for SolanaRegistrationError {
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        let mut detail = err_detail!(self);
        detail.insert(
            "message".to_string(),
            serde_json::to_value(self.to_string())
                .expect("Converting string to serde value should be infallible"),
        );
        Ok(detail)
    }
}

#[derive(Debug, PartialEq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
pub enum Event {
    UserRegistered { user_pubkey: [u8; 32], embedded_pubkey: [u8; 32] },
}

impl<S: Spec> Module for SolanaRegistration<S>
where
    S::Address: HyperlaneAddress,
{
    type Spec = S;

    type Error = SolanaRegistrationError;

    type Config = ();

    type CallMessage = ();

    type Event = Event;

    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: &Context<Self::Spec>,
        _state: &mut impl TxState<Self::Spec>,
    ) -> Result<(), Self::Error> {
        Err(SolanaRegistrationError::UnsupportedModuleCall)
    }
}

impl<S: Spec> Recipient<S> for SolanaRegistration<S>
where
    S::Address: HyperlaneAddress,
{
    fn ism(&self, recipient: &HexHash, state: &mut impl TxState<S>) -> anyhow::Result<Option<Ism>> {
        self.warp.ism(recipient, state)
    }

    fn default_ism(&self, state: &mut impl TxState<S>) -> anyhow::Result<Option<Ism>> {
        self.warp.default_ism(state)
    }

    fn handle(
        &mut self,
        origin: u32,
        sender: HexHash,
        recipient: &HexHash,
        body: HexString,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if self.should_handle(origin, sender) {
            Ok(self.register(body, state)?)
        } else {
            self.warp.handle(origin, sender, recipient, body, state)
        }
    }
}

impl<S: Spec> SolanaRegistration<S>
where
    S::Address: HyperlaneAddress,
{
    fn should_handle(&self, origin: u32, sender: HexHash) -> bool {
        let program_id = Base58Address::from_str(config::SOLANA_PROGRAM_ID).unwrap();
        origin == config::HYPERLANE_SOLANA_CHAIN_ID && sender == HexString(program_id.0)
    }

    fn unpack_body(&self, body: &[u8]) -> Result<([u8; 32], [u8; 32]), SolanaRegistrationError> {
        if body.len() < 64 {
            Err(SolanaRegistrationError::InvalidBodyLength {
                expected: 64,
                found: body.len(),
            })
        } else {
            let user_pubkey: [u8; 32] = body[0..32]
                .try_into()
                .map_err(|_| SolanaRegistrationError::ExtractPubKey)?;
            let embedded_pubkey: [u8; 32] = body[32..64]
                .try_into()
                .map_err(|_| SolanaRegistrationError::ExtractPubKey)?;
            Ok((user_pubkey, embedded_pubkey))
        }
    }

    fn register(
        &mut self,
        body: HexString,
        state: &mut impl TxState<S>,
    ) -> Result<(), SolanaRegistrationError> {
        let (user_pubkey, embedded_pubkey) = self.unpack_body(body.as_ref())?;
        let credential_id = CredentialId::from(embedded_pubkey);
        let address = S::Address::try_from(&user_pubkey).map_err(CoreModuleError::from)?;
        let resolved_address = self
            .accounts
            .resolve_sender_address(&address, &credential_id, state)
            .map_err(CoreModuleError::state_write)?;

        if address != resolved_address {
            Err(SolanaRegistrationError::AlreadyRegistered {
                attempted_address: address.to_string(),
                registered_address: resolved_address.to_string(),
            })
        } else {
            self.emit_event(state, Event::UserRegistered {
                user_pubkey,
                embedded_pubkey,
            });
            Ok(())
        }
    }
}

pub mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use crate::config;
    use crate::SolanaRegistration;
    use borsh::BorshDeserialize;
    use sov_modules_api::Base58Address;
    use sov_modules_api::HexHash;
    use sov_test_utils::TestSpec as S;

    fn b58_as_hex(s: &str) -> HexHash {
        let b58 = Base58Address::from_str(s).unwrap();
        HexHash::try_from_slice(&b58.0).unwrap()
    }

    fn valid_program_id() -> HexHash {
        b58_as_hex(config::SOLANA_PROGRAM_ID)
    }

    fn valid_domain() -> u32 {
        config::HYPERLANE_SOLANA_CHAIN_ID
    }

    #[test]
    fn test_should_handle() {
        let m = SolanaRegistration::<S>::default();

        assert!(
            !m.should_handle(5, valid_program_id()),
            "invalid domain should not be handled"
        );
        assert!(
            !m.should_handle(
                valid_domain(),
                b58_as_hex("692KZJaoe2KRcD6uhCQDLLXnLNA5ZLnfvdqjE4aX9iu1")
            ),
            "invalid program id should not be handled"
        );
        assert!(
            m.should_handle(valid_domain(), valid_program_id()),
            "should handle correct domain & program"
        );
    }

    #[test]
    fn test_unpack_body() {
        let payer = [1u8; 32];
        let embedded = [2u8; 32];
        let body = [payer, embedded].concat();
        // should return the tuple (first 32 bytes, second 32 bytes)
        let unpacked = SolanaRegistration::<S>::default()
            .unpack_body(&body)
            .unwrap();

        assert_eq!(unpacked.0, [1u8; 32]);
        assert_eq!(unpacked.1, [2u8; 32]);
    }
}
