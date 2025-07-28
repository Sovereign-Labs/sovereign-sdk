use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_modules_api::{Amount, DaSpec, GenesisState, Spec};

use crate::SequencerRegistry;

/// Genesis configuration for the [`SequencerRegistry`] module.
///
/// This `struct` must be passed as an argument to
/// [`Module::genesis`](sov_modules_api::Module::genesis).
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, JsonSchema)]
#[serde(bound = "S::Address: serde::Serialize + serde::de::DeserializeOwned")]
#[schemars(
    bound = "S: sov_modules_api::Spec, <S::Da as DaSpec>::Address: JsonSchema",
    rename = "SequencerRegistryConfig"
)]
pub struct SequencerRegistryConfig<S: Spec> {
    /// Registration will fail if the sequencer bonds less than this amount.
    pub minimum_bond: Amount,
    /// The initial configuration for the sequencer.
    pub sequencer_config: SequencerConfig<S>,
}

/// The initial sequencer config.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, JsonSchema)]
#[schemars(
    bound = "S: sov_modules_api::Spec, <S::Da as DaSpec>::Address: JsonSchema",
    rename = "SequencerConfig"
)]
pub struct SequencerConfig<S: Spec> {
    /// The rollup address of the sequencer.
    pub seq_rollup_address: S::Address,
    /// The Data Availability (DA) address of the sequencer.
    pub seq_da_address: <S::Da as DaSpec>::Address,
    /// Initial sequencer bond
    pub seq_bond: Amount,
    /// Determines whether this sequencer is *regular* or *preferred*.
    ///
    /// Batches from the preferred sequencer are always processed first in
    /// block, which means the preferred sequencer can guarantee soft
    /// confirmation time for transactions.
    pub is_preferred_sequencer: bool,
}

impl<S: Spec> SequencerRegistry<S> {
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as sov_modules_api::Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        self.minimum_bond.set(&config.minimum_bond, state)?;

        let config = &config.sequencer_config;

        tracing::info!(
            sequencer_rollup_address = %config.seq_rollup_address,
            sequencer_da_address = %config.seq_da_address,
            sequencer_bond = %config.seq_bond,
            is_preferred_sequencer = config.is_preferred_sequencer,
            "Starting sequencer registry genesis..."
        );

        self.register_staker(
            &config.seq_da_address,
            config.seq_bond,
            config.seq_rollup_address.clone(),
            state,
        )?;

        if config.is_preferred_sequencer {
            self.preferred_sequencer
                .set(&config.seq_da_address, state)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_mock_da::MockAddress;
    use sov_modules_api::prelude::*;
    use sov_modules_api::{AddressBech32, Amount};
    use sov_test_utils::TestSpec;

    use crate::SequencerConfig;

    #[test]
    fn test_config_serialization() {
        let seq_rollup_address: <TestSpec as Spec>::Address =
            AddressBech32::from_str("sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf")
                .unwrap()
                .into();

        let seq_da_address = MockAddress::from_str(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap();

        let config = SequencerConfig::<TestSpec> {
            seq_rollup_address,
            seq_da_address,
            seq_bond: Amount::new(100),
            is_preferred_sequencer: true,
        };

        let data = r#"
        {
            "seq_rollup_address":"sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
            "seq_da_address":"0000000000000000000000000000000000000000000000000000000000000000",
            "seq_bond":"100",
            "is_preferred_sequencer":true
        }"#;

        let parsed_config: SequencerConfig<TestSpec> = serde_json::from_str(data).unwrap();
        assert_eq!(config, parsed_config);
    }
}
