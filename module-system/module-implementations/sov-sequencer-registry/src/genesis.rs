use anyhow::Result;
use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::*;
use sov_modules_api::WorkingSet;

use crate::SequencerRegistry;

/// Genesis configuration for the [`SequencerRegistry`] module.
///
/// This `struct` must be passed as an argument to
/// [`Module::genesis`](sov_modules_api::Module::genesis).
///
// TODO: Should we allow multiple sequencers in genesis?
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(bound = "C::Address: serde::Serialize + serde::de::DeserializeOwned")]
pub struct SequencerConfig<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> {
    /// The rollup address of the sequencer.
    pub seq_rollup_address: C::Address,
    /// The Data Availability (DA) address of the sequencer.
    pub seq_da_address: Da::Address,
    /// Coins that will be slashed if the sequencer is malicious.
    ///
    /// The coins will be transferred from
    /// [`SequencerConfig::seq_rollup_address`] to this module's address
    /// ([`sov_modules_api::ModuleInfo::address`]) and locked away until the sequencer
    /// decides to exit (unregister).
    ///
    /// Only sequencers that are [`SequencerRegistry::is_sender_allowed`] list are
    /// allowed to exit.
    pub coins_to_lock: sov_bank::Coins<C>,
    /// Determines whether this sequencer is *regular* or *preferred*.
    ///
    /// Batches from the preferred sequencer are always processed first in
    /// block, which means the preferred sequencer can guarantee soft
    /// confirmation time for transactions.
    pub is_preferred_sequencer: bool,
}

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> SequencerRegistry<C, Da> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        self.coins_to_lock.set(&config.coins_to_lock, working_set);
        self.register_sequencer(
            &config.seq_da_address,
            &config.seq_rollup_address,
            working_set,
        )?;
        if config.is_preferred_sequencer {
            self.preferred_sequencer
                .set(&config.seq_da_address, working_set);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_bank::Coins;
    use sov_mock_da::{MockAddress, MockDaSpec};
    use sov_modules_api::default_context::DefaultContext;
    use sov_modules_api::{AddressBech32, Spec};

    use crate::SequencerConfig;

    #[test]
    fn test_config_serialization() {
        let seq_rollup_address: <DefaultContext as Spec>::Address = AddressBech32::from_str(
            "sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94",
        )
        .unwrap()
        .into();

        let token_address: <DefaultContext as Spec>::Address = AddressBech32::from_str(
            "sov1zsnx7n2wjvtkr0ttscfgt06pjca3v2e6stxeu49qwynavmk7a8xqlxkkjp",
        )
        .unwrap()
        .into();

        let coins = Coins::<DefaultContext> {
            amount: 50,
            token_address,
        };

        let seq_da_addreess = MockAddress::from_str(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap();

        let config = SequencerConfig::<DefaultContext, MockDaSpec> {
            seq_rollup_address,
            seq_da_address: seq_da_addreess,
            coins_to_lock: coins,
            is_preferred_sequencer: true,
        };

        let data = r#"
        {
            "seq_rollup_address":"sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94",
            "seq_da_address":"0000000000000000000000000000000000000000000000000000000000000000",
            "coins_to_lock":{
                "amount":50,
                "token_address":"sov1zsnx7n2wjvtkr0ttscfgt06pjca3v2e6stxeu49qwynavmk7a8xqlxkkjp"
            },
            "is_preferred_sequencer":true
        }"#;

        let parsed_config: SequencerConfig<DefaultContext, MockDaSpec> =
            serde_json::from_str(data).unwrap();
        assert_eq!(config, parsed_config)
    }
}
