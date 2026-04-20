//! This file implements traits only useful for testing.
//! It allows for compatibility of the [`Runtime`] with Sovereign's testing framework.
//! Users that want to fully use the testing framework with a custom runtime should implement these traits.
//! Note though that, in practice, the runtime trait (and the methods below) can be macro-derived using the
//! framework's macro exports. See `sov-test-utils` crate for additional information

use sov_address::{EthereumAddress, FromVmAddress};
use sov_hyperlane_integration::HyperlaneAddress;
use sov_modules_api::{Base58Address, Genesis, Spec};
use sov_sequencer_registry::SequencerRegistry;
use sov_test_utils::runtime::traits::MinimalGenesis;

use crate::runtime::Runtime;

impl<S: Spec> MinimalGenesis<S> for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    /// Returns a reference to the sequencer registry config.
    fn sequencer_registry_config(
        config: &Self::Config,
    ) -> &<SequencerRegistry<S> as Genesis>::Config {
        &config.sequencer_registry
    }
}
