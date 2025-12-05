use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{
    err_detail, Base58Address, Context, CoreModuleError, CredentialId, DaSpec, ErrorContext,
    ErrorDetail, EventEmitter, GenesisState, HexHash, HexString, Module, ModuleId, ModuleInfo,
    ModuleRestApi, Spec, StateValue, TxState,
};

use sov_hyperlane_integration::{HyperlaneAddress, Ism, Recipient, Warp};

#[derive(Debug, PartialEq, Eq, Clone, UniversalWallet, JsonSchema)]
#[serialize(Borsh, Serde)]
pub struct SolanaDeployment {
    /// The Solana hyperlane domain id.
    pub domain_id: u32,
    /// The program id of the hyperlane-solana-register program deployed on Solana.
    /// https://github.com/Sovereign-Labs/hyperlane-solana-register/tree/master/solana/program
    ///
    /// This is a TRUSTED program, the owner of this program can arbitrarily register users on the rollup
    /// which could lead to account takeovers if misused.
    pub program_id: Base58Address,
}

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

    #[state]
    admin: StateValue<S::Address>,

    #[state]
    deployment: StateValue<SolanaDeployment>,

    #[state]
    ism: StateValue<Ism>,
}

#[derive(thiserror::Error, Debug, serde::Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum SolanaRegistrationError {
    #[error("Core module error: {0}")]
    CoreModuleError(#[from] CoreModuleError),
    #[error("Embedded pubkey already registered to different address. Attempted: {attempted_address}, Registered: {registered_address}")]
    AlreadyRegistered {
        attempted_address: String,
        registered_address: String,
    },
    #[error("Invalid body length. Expected {expected}, found {found}")]
    InvalidBodyLength { expected: usize, found: usize },
    #[error("Failed to extract public key from body")]
    ExtractPubKey,
    #[error("Admin not set for module")]
    AdminNotFound,
    #[error("Module can only be called by the admin")]
    Forbidden { admin: String, caller: String },
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
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "Event")]
pub enum Event<S: Spec> {
    UserRegistered {
        address: S::Address,
        credential_id: CredentialId,
    },
    Updated {
        admin: Option<S::Address>,
        deployment: Option<SolanaDeployment>,
        ism: Option<Ism>,
    },
}

#[derive(Debug, Clone)]
#[serialize(Borsh, Serde)]
pub struct GenesisConfig<S: Spec> {
    pub deployment: Option<SolanaDeployment>,
    pub ism: Option<Ism>,
    pub admin: S::Address,
}

#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[schemars(bound = "S::Gas: ::schemars::JsonSchema", rename = "CallMessage")]
#[serde(rename_all = "snake_case")]
pub enum CallMessage<S: Spec> {
    Update {
        admin: Option<S::Address>,
        deployment: Option<SolanaDeployment>,
        ism: Option<Ism>,
    },
}

impl<S: Spec> Module for SolanaRegistration<S>
where
    S::Address: HyperlaneAddress,
{
    type Spec = S;

    type Error = SolanaRegistrationError;

    type Config = GenesisConfig<S>;

    type CallMessage = CallMessage<S>;

    type Event = Event<S>;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<(), anyhow::Error> {
        if let Some(deployment) = &config.deployment {
            self.deployment.set(deployment, state)?;
        }

        if let Some(ism) = &config.ism {
            self.ism.set(ism, state)?;
        }

        self.admin.set(&config.admin, state)?;

        Ok(())
    }

    fn call(
        &mut self,
        message: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<Self::Spec>,
    ) -> Result<(), Self::Error> {
        let admin = self
            .admin
            .get(state)
            .map_err(CoreModuleError::state_read)?
            .ok_or(SolanaRegistrationError::AdminNotFound)?;
        let sender = context.sender();
        self.assert_admin(sender, &admin)?;

        match message {
            CallMessage::Update {
                admin,
                deployment,
                ism,
            } => {
                self.update(admin, deployment, ism, state)?;
            }
        };

        Ok(())
    }
}

impl<S: Spec> Recipient<S> for SolanaRegistration<S>
where
    S::Address: HyperlaneAddress,
{
    fn ism(
        &self,
        _recipient: &HexHash,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<Option<Ism>> {
        self.default_ism(state)
    }

    fn default_ism(&self, state: &mut impl TxState<S>) -> anyhow::Result<Option<Ism>> {
        Ok(self.ism.get(state)?)
    }

    fn handle(
        &mut self,
        origin: u32,
        sender: HexHash,
        recipient: &HexHash,
        body: HexString,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let deployment = self.deployment.get(state)?.ok_or_else(|| {
            anyhow::anyhow!("SolanaDeployment not configured in SolanaRegistration module")
        })?;
        if self.should_handle(origin, sender, &deployment) {
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
    fn update(
        &mut self,
        admin: Option<S::Address>,
        deployment: Option<SolanaDeployment>,
        ism: Option<Ism>,
        state: &mut impl TxState<S>,
    ) -> Result<(), SolanaRegistrationError> {
        if let Some(new_admin) = &admin {
            self.admin
                .set(new_admin, state)
                .map_err(CoreModuleError::state_write)?;
        }

        if let Some(new_deployment) = &deployment {
            self.deployment
                .set(new_deployment, state)
                .map_err(CoreModuleError::state_write)?;
        }

        if let Some(new_ism) = &ism {
            self.ism
                .set(new_ism, state)
                .map_err(CoreModuleError::state_write)?;
        }

        self.emit_event(
            state,
            Event::Updated {
                admin,
                deployment,
                ism,
            },
        );

        Ok(())
    }

    fn assert_admin(
        &self,
        caller: &S::Address,
        admin: &S::Address,
    ) -> Result<(), SolanaRegistrationError> {
        if caller != admin {
            Err(SolanaRegistrationError::Forbidden {
                admin: admin.to_string(),
                caller: caller.to_string(),
            })
        } else {
            Ok(())
        }
    }

    fn should_handle(&self, origin: u32, sender: HexHash, deploy: &SolanaDeployment) -> bool {
        origin == deploy.domain_id && sender == HexString(deploy.program_id.0)
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
            self.emit_event(
                state,
                Event::UserRegistered {
                    address,
                    credential_id,
                },
            );
            Ok(())
        }
    }

    pub fn admin(
        &self,
        state: &mut impl TxState<S>,
    ) -> Result<Option<S::Address>, SolanaRegistrationError> {
        Ok(self.admin.get(state).map_err(CoreModuleError::state_read)?)
    }

    pub fn deployment(
        &self,
        state: &mut impl TxState<S>,
    ) -> Result<Option<SolanaDeployment>, SolanaRegistrationError> {
        Ok(self
            .deployment
            .get(state)
            .map_err(CoreModuleError::state_read)?)
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use crate::SolanaDeployment;
    use crate::SolanaRegistration;
    use borsh::BorshDeserialize;
    use sov_modules_api::Base58Address;
    use sov_modules_api::HexHash;
    use sov_test_utils::TestSpec as S;

    fn b58_as_hex(s: &str) -> HexHash {
        let b58 = Base58Address::from_str(s).unwrap();
        HexHash::try_from_slice(&b58.0).unwrap()
    }

    #[test]
    fn test_should_handle() {
        let valid_program_id = "692KZJaoe2KRcD6uhCQDLLXnLNA5ZLnfvdqjE4aX9iu1";
        let deploy = SolanaDeployment {
            domain_id: 1337,
            program_id: Base58Address::from_str(valid_program_id).unwrap(),
        };
        let m = SolanaRegistration::<S>::default();

        assert!(
            !m.should_handle(5, b58_as_hex(valid_program_id), &deploy),
            "invalid domain should not be handled"
        );
        assert!(
            !m.should_handle(
                1337,
                b58_as_hex("692KZJaoe2KRcD6uhCQDLLXnLNA5ZLnfvdqjE4aX9i22"),
                &deploy,
            ),
            "invalid program id should not be handled"
        );
        assert!(
            m.should_handle(1337, b58_as_hex(valid_program_id), &deploy),
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
