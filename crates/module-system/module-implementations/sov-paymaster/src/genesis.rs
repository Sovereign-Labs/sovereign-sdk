use anyhow::Result;
use serde::{Deserialize, Serialize};
use sov_modules_api::{DaSpec, GenesisState, Module, Spec};

use crate::call::SafeVec;
use crate::{Paymaster, PaymasterPolicyInitializer};

/// The genesis configuration of the paymaster module, consisting of a list of
/// payers and their policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", bound = "S: Spec")]
pub struct PaymasterConfig<S: Spec> {
    #[allow(missing_docs)]
    // We set a conservative limit of 5 payers to prevent stack overflows, since
    // `SafeVec` is stack allocated and PayerGenesisConfig has nested SafeVecs
    pub payers: SafeVec<PayerGenesisConfig<S>, 5>,
}

impl<S: Spec> Default for PaymasterConfig<S> {
    fn default() -> Self {
        Self {
            payers: SafeVec::new(),
        }
    }
}

/// The genesis config for a particular payer. Unlike standard payer registration,
/// the genesis config for a payer needs to explicitly list which sequencers should
/// use the payer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "S: Spec")]
pub struct PayerGenesisConfig<S: Spec> {
    /// The address of the newly registered payer.
    pub payer_address: S::Address,
    /// The policy that this payer will use.
    pub policy: PaymasterPolicyInitializer<S>,
    /// The list of sequencers that should be configured to use this payer after genesis. Any sequencers in this
    /// list must also be authorized by the payer's policy.
    pub sequencers_to_register: SafeVec<<S::Da as DaSpec>::Address>,
}

impl<S: Spec> Paymaster<S> {
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        for PayerGenesisConfig {
            payer_address,
            policy,
            sequencers_to_register,
        } in &config.payers
        {
            self.do_registration(
                payer_address,
                sequencers_to_register.iter(),
                policy.clone(),
                state,
            )?;
        }
        Ok(())
    }
}
