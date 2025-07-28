#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod call;
mod event;
mod genesis;
mod policies;
use std::fmt::Display;
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
pub use call::*;
pub use event::Event;
pub use genesis::*;
pub use policies::*;
use sov_bank::ReserveGasError;
use sov_modules_api::transaction::AuthenticatedTransactionData;
use sov_modules_api::{
    Context, DaSpec, Gas, GenesisState, InnerEnumVariant, Module, ModuleId, ModuleInfo,
    ModuleRestApi, Spec, StateAccessor, StateMap, TxState,
};
use sov_state::{BorshCodec, EncodeLike};

/// The key to a policy, consisting of the payer and payee addresses with a separator.
#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    derive_more::Display,
)]
#[display(r#"payers/{}{POLICY_SEPARATOR}{}"#, self.payer, self.payee)]
pub struct PolicyKey<Address: Display> {
    payer: Address,
    payee: Address,
}
const POLICY_SEPARATOR: &str = "/policy/";

impl<Address: Display + FromStr<Err: Into<Box<dyn std::error::Error + Send + Sync + 'static>>>>
    FromStr for PolicyKey<Address>
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some(s) = s.strip_prefix("payers/") else {
            anyhow::bail!("{} is not a policykey - missing 'payers/' prefix", s);
        };
        // There's an extremely nasty edge case here where the string `/policy/` is allowed as part of an address.
        // (This is the case, for example, for base64 addresses). In this case, an adversary could pretty easily grind
        // an address which contains the /policy/ string. To handle this edge case, we have to iteratively try every split
        // until we find one that works. This assumes that the address type is constrained by length
        for (idx, _) in s.match_indices("/policy/") {
            let payer = &s[..idx];
            let payee = &s[idx + POLICY_SEPARATOR.len()..];
            let payer = Address::from_str(payer);
            if let Ok(payer) = payer {
                let payee =
                    Address::from_str(payee).map_err(|e| anyhow::Error::from_boxed(e.into()))?;
                return Ok(PolicyKey::with(Payer(payer), payee));
            }
        }
        anyhow::bail!("{} could not be parsed as a policy key", s);
    }
}

struct Payer<T>(pub T);

impl<Addr: Display> PolicyKey<Addr> {
    fn with(payer: Payer<Addr>, payee: Addr) -> Self {
        Self {
            payer: payer.0,
            payee,
        }
    }
}

impl<'a, Addr: Display + BorshSerialize + BorshDeserialize>
    EncodeLike<(Payer<&'a Addr>, &Addr), PolicyKey<Addr>> for BorshCodec
{
    fn encode_like(&self, borrowed: &(Payer<&'a Addr>, &Addr)) -> Vec<u8> {
        let mut out = self.encode_like(borrowed.0 .0);
        out.extend_from_slice(&self.encode_like(borrowed.1));
        out
    }
}

/// The `Paymaster` module allows a third party to gas on behalf of a user.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
#[module_info(sequencer_safety = "is_safe_for_sequencer")]
pub struct Paymaster<S: Spec> {
    /// Id of the module.
    #[id]
    pub id: ModuleId,

    /// A mapping from paymaster addresses to their policies.
    #[state]
    pub payers: StateMap<S::Address, PaymasterPolicy<S>>,

    /// Policies per user
    #[state]
    pub policies: StateMap<PolicyKey<S::Address>, PayeePolicy<S>>,

    /// Maps the sequencer to the appropriate gas payer.
    #[state]
    pub sequencer_to_payer: StateMap<<S::Da as DaSpec>::Address, S::Address>,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<S>,
}

fn is_safe_for_sequencer<S: Spec>(
    _module: &Paymaster<S>,
    call: InnerEnumVariant<'_>,
    sequencer_address: &<S::Da as DaSpec>::Address,
) -> bool {
    if let Some(call) = call.inner().downcast_ref::<CallMessage<S>>() {
        // Updates are unsafe if they could change the payer registered for this sequencer
        match call {
            CallMessage::RegisterPaymaster { policy } => {
                // If a policy doesn't include this sequencer, we're fine
                !policy.authorized_sequencers.covers(sequencer_address)
            }
            // A set payer message is always dangerous
            //
            // Similarly, any policy update could change this sequencer's payer since the sequencer may already
            // be an authorized sequencer on that policy.
            // TODO: Change the update rules so that the sequencer is changed only if *newly* whitlisted.
            // Then, update this sequencer safety implementation
            CallMessage::SetPayerForSequencer { .. } | CallMessage::UpdatePolicy { .. } => false,
        }
    } else {
        // Calls to other modules are safe as far as we're concerned
        true
    }
}

impl<S: Spec> Module for Paymaster<S> {
    type Spec = S;

    type Config = genesis::PaymasterConfig<S>;

    type CallMessage = CallMessage<S>;

    type Event = Event<S>;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        // The initialization logic
        self.init_module(config, state)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            CallMessage::RegisterPaymaster { policy } => {
                self.register_paymaster(policy, context, state)?;
            }
            CallMessage::SetPayerForSequencer { payer } => {
                self.set_payer_for_sequencer(payer, context, state)?;
            }
            CallMessage::UpdatePolicy { payer, update } => {
                self.update_policy_if_authorized(update, context, &payer, state)?;
            }
        }
        Ok(())
    }
}

impl<S: Spec> Paymaster<S> {
    /// Get the payer's policy pertaining to the current `context.sender()`
    fn get_payee_policy_if_sequencer_authorized(
        &self,
        payer: &S::Address,
        context: &Context<S>,
        state: &mut impl StateAccessor,
    ) -> Result<Option<PayeePolicy<S>>, ReserveGasError> {
        let policy = self
            .payers
            .get(payer, state)
            .map_err(|e| ReserveGasError::StateAccessError(e.to_string()))?;

        let Some(policy) = policy else {
            return Ok(None);
        };

        if !policy
            .authorized_sequencers
            .covers(context.sequencer_da_address())
        {
            return Ok(None);
        }
        Ok(Some(
            self.policies
                .get(&(Payer(payer), context.sender()), state)
                .map_err(|e| ReserveGasError::StateAccessError(e.to_string()))?
                .unwrap_or(policy.default_payee_policy),
        ))
    }

    /// Try to reserve gas for the transaction using the payer's balance if possible and falling back to the sender's balance if not.
    pub fn try_reserve_gas(
        &mut self,
        tx: &AuthenticatedTransactionData<S>,
        gas_price: &<S::Gas as Gas>::Price,
        context: &mut Context<S>,
        state: &mut impl StateAccessor,
    ) -> Result<(), ReserveGasError> {
        if let Some(payer) = self.gas_from_paymaster(tx, gas_price, context, state)? {
            context.set_gas_refund_recipient(payer);
            return Ok(());
        }
        // If the paymaster will not pay for whatever reason, the user pays.
        // This prevents someone from censoring transactions by setting overly strict payer policies which cause
        // them to be rejected.
        tracing::trace!("Falling back to user balance to reserve gas");
        self.bank
            .reserve_gas(tx, gas_price, context.sender(), state)?;
        context.set_gas_refund_recipient(context.sender().clone());
        Ok(())
    }

    /// Reserves the entire available gas amount from the paymaster if the paymaster's policy permits it
    /// and it has sufficient balance. Otherwise, reserves no gas and returns None.
    fn gas_from_paymaster(
        &mut self,
        tx: &AuthenticatedTransactionData<S>,
        gas_price: &<S::Gas as Gas>::Price,
        context: &Context<S>,
        state: &mut impl StateAccessor,
    ) -> Result<Option<S::Address>, ReserveGasError> {
        let payer = self
            .sequencer_to_payer
            .get(context.sequencer_da_address(), state)
            .map_err(|e| ReserveGasError::StateAccessError(e.to_string()))?;

        let Some(payer) = payer else { return Ok(None) };

        // If this sequencer acts as a paymaster, it reserves the gas for the transaction.
        if let Some(payee_policy) =
            self.get_payee_policy_if_sequencer_authorized(&payer, context, state)?
        {
            // If the paymaster pays for the gas, it also needs to get the refund
            match self.try_purchase_paymaster_gas(tx, gas_price, &payer, &payee_policy, state) {
                Ok(Some(mutated_policy)) => {
                    self.policies
                        .set(&(Payer(&payer), context.sender()), &mutated_policy, state)
                        .map_err(|e| ReserveGasError::StateAccessError(e.to_string()))?;
                    return Ok(Some(payer));
                }
                Ok(None) => {
                    return Ok(Some(payer));
                }
                Err(e) => {
                    tracing::debug!(reason = %e, "Failed to pay gas using paymaster");
                }
            }
        } else {
            // If the sequencer isn't authorized for this payer, our map is stale. Remove the stale entry
            self.sequencer_to_payer
                .remove(context.sequencer_da_address(), state)
                .map_err(|e| ReserveGasError::StateAccessError(e.to_string()))?;

            tracing::debug!(
                sequencer = %context.sequencer_da_address(),
                attempted_payer = %payer,
                "Sequencer is not authorized for payer. Removing sequencer_to_payer entry.",
            );
        }
        Ok(None)
    }

    /// Purchases the gas for the transaction using the payer's balance if possible.
    fn try_purchase_paymaster_gas(
        &mut self,
        tx: &AuthenticatedTransactionData<S>,
        gas_price: &<S::Gas as Gas>::Price,
        payer: &S::Address,
        policy: &PayeePolicy<S>,
        state: &mut impl StateAccessor,
    ) -> Result<Option<PayeePolicy<S>>, ReserveGasError> {
        let maybe_mutated_policy = policy.authorize_transaction(tx, gas_price)?;
        self.bank.reserve_gas(tx, gas_price, payer, state)?;
        Ok(maybe_mutated_policy)
    }
}

#[test]
fn test_policy_key_encode_like() {
    let key = PolicyKey::with(Payer(1), 2);
    let encoded_like = BorshCodec.encode_like(&(Payer(&key.payer), &key.payee));

    assert_eq!(&borsh::to_vec(&key).unwrap(), &encoded_like);
}

#[test]
fn test_policy_key_from_str() {
    #[derive(PartialEq, Eq, Debug, Clone)]
    struct MaliciousAddress([u8; 12]);

    impl From<&[u8; 12]> for MaliciousAddress {
        fn from(value: &[u8; 12]) -> Self {
            Self(*value)
        }
    }

    impl FromStr for MaliciousAddress {
        type Err = anyhow::Error;
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            if s.len() == 12 {
                return Ok(Self(s.as_bytes().try_into().unwrap()));
            }
            anyhow::bail!("String must be 12 bytes!");
        }
    }

    impl Display for MaliciousAddress {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&String::from_utf8_lossy(&self.0))
        }
    }

    // Test roundtrip for a *valid* address that contains the separator at an irrelelvant location
    let key =
        PolicyKey::<MaliciousAddress>::with(Payer(b"evil/policy/".into()), b"innocuouskey".into());
    let key_str = format!("{key}");
    assert_eq!(
        &format!("payers/evil/policy/{POLICY_SEPARATOR}innocuouskey"),
        &key_str
    );
    let recovered_key = PolicyKey::from_str(&key_str).expect("Valid key must deserialize!");
    assert_eq!(recovered_key, key);

    // Test an invalid address that contains lots of the separator
    assert!(PolicyKey::<MaliciousAddress>::from_str(&format!(
        "{POLICY_SEPARATOR}{POLICY_SEPARATOR}{POLICY_SEPARATOR}{POLICY_SEPARATOR}"
    ))
    .is_err());
}
