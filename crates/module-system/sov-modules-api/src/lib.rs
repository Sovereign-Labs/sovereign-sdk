//! This crate defines the core traits and types used by the module system of Sovereign's SDK.
//!
//! It specifies interfaces which allow to communicate with the rollup storage and state (state accessors from the `state module`),
//! the module state (defined in `containers` and `module`) but also provides tools to interact/query the rollup (CLI, RPC, REST API, ...).
//! We also define an interface to handle and process transactions inside the `transaction` module.
//! General utilities used throughout the codebase are available inside the `common` module.

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod batch;
/// Defines interfaces to interact with the rollup's state through CLI.
#[cfg(feature = "native")]
pub mod cli;

/// Defines common types, concepts and utilities used throughout the codebase.
pub mod common;

/// Defines general containers types to interact with the modules' state.
mod containers;

/// Defines gas traits and metered utilities for gas accounting in the module system.
pub mod gas;

/// Defines default types and trait implementations for rollups built with the SDK
pub mod default_spec;

/// Defines a more configurable rollup spec
pub mod configurable_spec;

/// SDK internals. This module contains utilities for working with higher-kinded types and traits.
pub mod higher_kinded_types;

/// Defines hooks that can be used within the module system.
pub mod hooks;

/// Defines the module trait and its associated types. More generally, this module defines the
/// way modules are represented in the SDK.
pub mod module;

/// Library for stake registration functionality.
pub mod registration_lib;

/// Defines traits and utilities for writing REST(ful) APIs that expose rollup data.
#[cfg(feature = "native")]
pub mod rest;

/// Defines traits for interacting with the ledger when using the sov module system.
#[cfg(feature = "native")]
pub mod rpc;

/// Defines the interfaces to interact with the rollup's runtime, more specifically the capabilities
/// that they may implemented and their possible interactions with the kernel.
pub mod runtime;

/// Defines the interfaces to interact with the rollup's state, more specifically the state accessors and event handlers/emitters.
pub mod state;

/// Defines the metadata that is used to generate execution proofs.
pub mod proof_metadata;

mod reexport_macros;

#[cfg(test)]
mod tests;

/// Defines the transaction traits and utilities for working with transactions.
pub mod transaction;

/// Reexports traits and utilities for optimistic rollups.
pub mod optimistic {
    pub use sov_rollup_interface::optimistic::{Attestation, BondingProofService, ProofOfBond};
}

/// Reexports traits and utilities for DA layers.
pub mod da {
    pub use sov_rollup_interface::da::{BlockHeaderTrait, Time};
}

/// Types related to receipts.
mod tx_receipt;

use std::collections::{HashMap, HashSet};

pub use batch::*;
#[cfg(feature = "native")]
pub use clap;
pub use common::*;
pub use containers::*;
pub use gas::*;
pub use hooks::*;
pub use module::*;
pub use reexport_macros::*;
#[cfg(feature = "native")]
pub use rpc::*;
pub use runtime::*;
pub use sov_rollup_interface::common::{
    safe_vec, HexHash, HexString, SafeString, SafeVec, SizedSafeString, VisibleSlotNumber,
};
#[cfg(feature = "native")]
pub use sov_rollup_interface::crypto::PrivateKey;
pub use sov_rollup_interface::crypto::{CredentialId, PublicKey, Signature};
pub use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
#[cfg(feature = "native")]
pub use sov_rollup_interface::node::da::SlotData;
#[cfg(feature = "native")]
pub use sov_rollup_interface::node::{DaSyncState, SyncStatus};
pub use sov_rollup_interface::optimistic::{SerializedAttestation, SerializedChallenge};
pub use sov_rollup_interface::reexports::digest;
pub use sov_rollup_interface::stf::{
    ApplySlotOutput, BatchReceipt, ExecutionContext, IgnoredTransactionReceipt, InvalidProofError,
    ProofOutcome, ProofReceipt, ProofReceiptContents, ProofSender, StateTransitionFunction,
    StoredEvent,
};
pub use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, CodeCommitment, SerializedAggregatedProof,
};
#[cfg(feature = "native")]
pub use sov_rollup_interface::zk::HostArgs;
pub use sov_rollup_interface::zk::{
    CodeCommitmentFor, CryptoSpec, StateTransitionPublicData, ZkVerifier, Zkvm,
};
#[cfg(feature = "native")]
pub use sov_rollup_interface::StateUpdateInfo;
pub use sov_rollup_interface::{execution_mode, BasicAddress, TxHash};
pub use sov_state::{CompileTimeNamespace, Storage};
pub use state::*;
pub use transaction::AuthenticatedTransactionData;
pub use tx_receipt::*;
pub use {schemars, sov_universal_wallet};

pub use crate::common::ModuleError as Error;
pub use crate::state::StateReaderAndWriter;

/// Prelude with re-exports of external crates used by macros, as well as
/// important traits and types.
///
/// This is meant to be "glob" imported wherever you'll use
/// [`sov_modules_api::macros`](crate::macros).
///
/// ```rust
/// use sov_modules_api::prelude::*;
/// use sov_modules_api::macros::UniversalWallet;
///
/// #[derive(UniversalWallet)]
/// struct MyStruct;
/// ```
pub mod prelude {
    // A NOTE ABOUT PRELUDES
    // ---------------------
    // I'm generally against preludes in Rust code, and the rest of the Rust
    // community seems to agree. I believe our use case warrants a prelude, though,
    // as we reexport many macros from many different crates, and a "glob" import is
    // the only way for us to inject all of these dependencies into downstream code
    // without its authors having to mess with their Cargo manifests.
    //
    // There's another thing to consider. Oftentimes, downstream code will only ever
    // use proc-macros when the `native` Cargo feature is enabled, which would mean
    // that the prelude "glob" import generates an "unused import" warning when
    // `native` is disabled. To avoid this, it's a good idea for us to also re-export
    // some other items that will (almost) always be used regardless of Cargo
    // features. `Spec`, for example, fits the bill. This ensures the lowest
    // possible amount of warnings, and thus the amount of linting exceptions
    // that users will have to deal with.
    //
    // In some cases, it's easy for proc-macros to depend on dependencies
    // re-exported here. In others, however, the original proc-macro references
    // its dependency with an absolute path, e.g. `::serde`. There is,
    // unfortunately, no way around that, unless said proc-macro also allows
    // configuring the crate path, e.g.
    // `#[serde(crate = "sov_modules_api::prelude::serde")]`
    //
    // This means that, in practice, re-exporting proc-macros is often difficult
    // or can't always be done.

    pub use crate::macros::*;
    #[cfg(feature = "native")]
    pub use crate::rest::ModuleRestApi;
    pub use crate::state::StateProvider;
    pub use crate::{
        Context, DaSpec, ModuleCallJsonSchema, Spec, StateAccessor, StateReaderAndWriter,
        WorkingSet,
    };
    pub extern crate tracing;

    pub extern crate anyhow;
    #[cfg(feature = "arbitrary")]
    pub extern crate arbitrary;
    #[cfg(feature = "native")]
    pub extern crate axum;
    pub extern crate bech32;
    #[cfg(feature = "native")]
    pub extern crate clap;
    #[cfg(feature = "native")]
    pub extern crate jsonrpsee;
    #[cfg(feature = "arbitrary")]
    pub extern crate proptest;
    #[cfg(feature = "arbitrary")]
    pub extern crate proptest_derive;
    pub extern crate schemars;
    pub extern crate serde;
    #[cfg(feature = "native")]
    pub extern crate serde_json;
    #[cfg(feature = "native")]
    pub extern crate serde_yaml;
    #[cfg(feature = "native")]
    pub extern crate sov_rest_utils;
    pub extern crate sov_universal_wallet;
    pub extern crate strum;
    #[cfg(feature = "native")]
    pub extern crate tokio;
    pub extern crate toml;
    #[cfg(feature = "native")]
    pub extern crate utoipa;
    #[cfg(feature = "native")]
    pub extern crate utoipa_swagger_ui;

    #[cfg(feature = "test-utils")]
    pub extern crate tracing_test;
    #[cfg(feature = "test-utils")]
    pub use tracing_test::traced_test;
    pub use unwrap_infallible::UnwrapInfallible;
}

struct ModuleVisitor<'a, S: Spec> {
    visited: HashSet<&'a ModuleId>,
    visited_on_this_path: Vec<&'a ModuleId>,
    sorted_modules: std::vec::Vec<&'a dyn ModuleInfo<Spec = S>>,
}

impl<'a, S: Spec> ModuleVisitor<'a, S> {
    pub fn new() -> Self {
        Self {
            visited: HashSet::new(),
            sorted_modules: Vec::new(),
            visited_on_this_path: Vec::new(),
        }
    }

    /// Visits all the modules and their dependencies, and populates a Vec of modules sorted by their dependencies
    fn visit_modules(&mut self, modules: Vec<&'a dyn ModuleInfo<Spec = S>>) -> anyhow::Result<()> {
        let mut module_map = HashMap::new();

        for module in &modules {
            module_map.insert(module.id(), *module);
        }

        for module in modules {
            self.visited_on_this_path.clear();
            self.visit_module(module, &module_map)?;
        }

        Ok(())
    }

    /// Visits a module and its dependencies, and populates a Vec of modules sorted by their dependencies
    fn visit_module(
        &mut self,
        module: &'a dyn ModuleInfo<Spec = S>,
        module_map: &HashMap<&'a ModuleId, &'a (dyn ModuleInfo<Spec = S>)>,
    ) -> anyhow::Result<()> {
        let id = module.id();

        // if the module have been visited on this path, then we have a cycle dependency
        if let Some((index, _)) = self
            .visited_on_this_path
            .iter()
            .enumerate()
            .find(|(_, &x)| x == id)
        {
            let cycle = &self.visited_on_this_path[index..];

            anyhow::bail!(
                "Cyclic dependency of length {} detected: {:?}",
                cycle.len(),
                cycle
            );
        }

        self.visited_on_this_path.push(id);

        // if the module hasn't been visited yet, visit it and its dependencies
        if self.visited.insert(id) {
            for dependency_address in module.dependencies() {
                let dependency_module = *module_map.get(&dependency_address).ok_or_else(|| {
                    anyhow::Error::msg(format!("Module not found: {dependency_address:?}"))
                })?;
                self.visit_module(dependency_module, module_map)?;
            }

            self.sorted_modules.push(module);
        }

        // remove the module from the visited_on_this_path list
        self.visited_on_this_path.pop();

        Ok(())
    }
}

/// Sorts `ModuleInfo` objects by their dependencies
fn sort_modules_by_dependencies<S: Spec>(
    modules: Vec<&dyn ModuleInfo<Spec = S>>,
) -> anyhow::Result<Vec<&dyn ModuleInfo<Spec = S>>> {
    let mut module_visitor = ModuleVisitor::<S>::new();
    module_visitor.visit_modules(modules)?;
    Ok(module_visitor.sorted_modules)
}

/// Accepts `Vec<>` of tuples `(&ModuleInfo, &TValue)`, and returns `Vec<&TValue>` sorted by mapped module dependencies
///
/// # Errors
/// Returns an error if any modules share `module_id`. Duplicate instances of the same module are
/// not allowed in a runtime.
pub fn sort_values_by_modules_dependencies<S: Spec, TValue>(
    module_value_tuples: Vec<(&dyn ModuleInfo<Spec = S>, TValue)>,
) -> anyhow::Result<Vec<TValue>>
where
    TValue: Clone,
{
    let sorted_modules = sort_modules_by_dependencies(
        module_value_tuples
            .iter()
            .map(|(module, _)| *module)
            .collect(),
    )?;

    let mut value_map = HashMap::new();

    for module in module_value_tuples {
        let prev_entry = value_map.insert(module.0.id(), module.1);
        anyhow::ensure!(prev_entry.is_none(), "Duplicate module id! Only one instance of each module is allowed in a given runtime. Module with ID {} is duplicated", module.0.id());
    }

    let mut sorted_values = Vec::new();
    for module in sorted_modules {
        // Unwrap: we just inserted the module_id above. Can only panic if
        // sort_modules_by_dependencies adds a new module_id that didn't exist in the input
        sorted_values.push(value_map.get(&module.id()).unwrap().clone());
    }

    Ok(sorted_values)
}

/// A trait that needs to be implemented for a *runtime* to be used with the CLI wallet
#[cfg(feature = "native")]
pub trait CliWallet: DispatchCall {
    /// The type that is used to represent this type in the CLI. Typically,
    /// this type implements the clap::Subcommand trait. This type is generic to
    /// allow for different representations of the same type in the interface; a
    /// typical end-usage will impl traits only in the case where `CliStringRepr<T>: Into::RuntimeCall`
    type CliStringRepr<T>;
}

#[doc(hidden)]
#[cfg(feature = "native")]
pub mod __rpc_macros_private {
    use rest::ApiState;

    use super::*;

    /// A [`Module`] that also exposes a JSON-RPC server.
    pub trait ModuleWithRpcServer {
        type Spec: Spec;

        fn rpc_methods(&self, state: ApiState<Self::Spec>) -> jsonrpsee::RpcModule<()>;
    }

    // Auto-ref trick so that implementing JSON-RPC for modules is effectively
    // optional.
    impl<M> ModuleWithRpcServer for &M
    where
        M: ModuleInfo,
    {
        type Spec = M::Spec;

        fn rpc_methods(&self, _state: ApiState<Self::Spec>) -> jsonrpsee::RpcModule<()> {
            jsonrpsee::RpcModule::new(())
        }
    }
}

/// Expands the given code block only when the `sov-modules-api/native` feature
/// is enabled.
#[cfg(feature = "native")]
#[macro_export]
macro_rules! native_only {
    ($($code:tt)*) => {
        $($code)*
    };
}

/// Expands the given code block only when the `sov-modules-api/native` feature
/// is enabled.
#[cfg(not(feature = "native"))]
#[macro_export]
macro_rules! native_only {
    ($($code:tt)*) => {};
}

/// Interprets usize as u32, panicking if it overflows.
pub fn as_u32_or_panic(val: usize) -> u32 {
    val.try_into()
        .expect("Overflow: Unable to cast usize to u32.")
}
