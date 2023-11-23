use sov_modules_api::Context;
use sov_rollup_interface::da::DaSpec;

#[cfg(feature = "cli")]
pub mod cli;

/// Blueprint definition for a module wallet.
///
/// The associated types of this trait are expected to be implemented concretely as binary
/// endpoints depends on resolved generics in order to compile properly.
///
/// The `DefaultWalletBlueprint` contains concrete types for the default implementations on
/// `sov-modules-api`. It lives behind a feature flag 'default-impl'.
pub trait WalletBlueprint {
    /// Context used to define the asymetric cryptography for keys generation and signing.
    type Context: Context;
    /// DA specification used to declare runtime call message of the module, that is signed by the
    /// wallet.
    type DaSpec: DaSpec;
}

#[cfg(feature = "default-impl")]
pub struct DefaultWalletBlueprint;

#[cfg(feature = "default-impl")]
impl WalletBlueprint for DefaultWalletBlueprint {
    type Context = sov_modules_api::default_context::ZkDefaultContext;
    type DaSpec = sov_mock_da::MockDaSpec;
}
