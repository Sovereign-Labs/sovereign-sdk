//! Provides traits which are useful for wrapping a (possibly incomplete) runtime implementation to create a test runtime
//! with configurable hooks.

use sov_modules_api::{Genesis, Spec};
use sov_sequencer_registry::SequencerRegistry;

/// A trait which allows access to the contents of the genesis configuration
/// for a runtime which implements [`Genesis`].
pub trait MinimalGenesis<S: Spec>: Genesis<Spec = S> {
    /// Returns a reference to the sequencer registry config.
    fn sequencer_registry_config(
        config: &Self::Config,
    ) -> &<SequencerRegistry<S> as Genesis>::Config;
}
