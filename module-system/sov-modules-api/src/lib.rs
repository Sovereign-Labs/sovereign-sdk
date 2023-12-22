#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
pub mod cli;
mod containers;
pub mod default_context;
pub mod default_signature;
pub mod hooks;
mod pub_key_hex;

#[cfg(feature = "macros")]
mod reexport_macros;
#[cfg(feature = "macros")]
pub use reexport_macros::*;

mod serde_pub_key;
#[cfg(test)]
mod tests;
pub mod transaction;
#[cfg(feature = "native")]
pub mod utils;

pub use containers::*;
pub use pub_key_hex::PublicKeyHex;
#[cfg(feature = "macros")]
extern crate sov_modules_macros;

use std::collections::{HashMap, HashSet};

#[cfg(feature = "native")]
pub use clap;
#[cfg(feature = "native")]
pub use sov_modules_core::PrivateKey;
pub use sov_modules_core::{
    archival_state, runtime, AccessoryWorkingSet, Address, AddressBech32, CallResponse, Context,
    DispatchCall, EncodeCall, GasUnit, Genesis, KernelModule, KernelWorkingSet, Module,
    ModuleCallJsonSchema, ModuleError, ModuleError as Error, ModuleInfo, ModulePrefix, PublicKey,
    Signature, Spec, StateCheckpoint, StateReaderAndWriter, VersionedWorkingSet, WorkingSet,
};
pub use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
pub use sov_rollup_interface::services::da::SlotData;
pub use sov_rollup_interface::stf::Event;
pub use sov_rollup_interface::zk::{
    StateTransition, ValidityCondition, ValidityConditionChecker, Zkvm,
};
pub use sov_rollup_interface::{digest, BasicAddress, RollupAddress};

pub mod prelude {
    pub use super::{StateMapAccessor, StateValueAccessor, StateVecAccessor};
}

pub mod optimistic {
    pub use sov_rollup_interface::optimistic::{Attestation, ProofOfBond};
}

pub mod da {
    pub use sov_rollup_interface::da::{BlockHeaderTrait, NanoSeconds, Time};
}

pub mod storage {
    pub use sov_rollup_interface::storage::HierarchicalStorageManager;
}

struct ModuleVisitor<'a, C: Context> {
    visited: HashSet<&'a <C as Spec>::Address>,
    visited_on_this_path: Vec<&'a <C as Spec>::Address>,
    sorted_modules: std::vec::Vec<&'a dyn ModuleInfo<Context = C>>,
}

impl<'a, C: Context> ModuleVisitor<'a, C> {
    pub fn new() -> Self {
        Self {
            visited: HashSet::new(),
            sorted_modules: Vec::new(),
            visited_on_this_path: Vec::new(),
        }
    }

    /// Visits all the modules and their dependencies, and populates a Vec of modules sorted by their dependencies
    fn visit_modules(
        &mut self,
        modules: Vec<&'a dyn ModuleInfo<Context = C>>,
    ) -> Result<(), anyhow::Error> {
        let mut module_map = HashMap::new();

        for module in &modules {
            module_map.insert(module.address(), *module);
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
        module: &'a dyn ModuleInfo<Context = C>,
        module_map: &HashMap<&<C as Spec>::Address, &'a (dyn ModuleInfo<Context = C>)>,
    ) -> Result<(), anyhow::Error> {
        let address = module.address();

        // if the module have been visited on this path, then we have a cycle dependency
        if let Some((index, _)) = self
            .visited_on_this_path
            .iter()
            .enumerate()
            .find(|(_, &x)| x == address)
        {
            let cycle = &self.visited_on_this_path[index..];

            anyhow::bail!(
                "Cyclic dependency of length {} detected: {:?}",
                cycle.len(),
                cycle
            );
        } else {
            self.visited_on_this_path.push(address)
        }

        // if the module hasn't been visited yet, visit it and its dependencies
        if self.visited.insert(address) {
            for dependency_address in module.dependencies() {
                let dependency_module = *module_map.get(dependency_address).ok_or_else(|| {
                    anyhow::Error::msg(format!("Module not found: {:?}", dependency_address))
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

/// Sorts ModuleInfo objects by their dependencies
fn sort_modules_by_dependencies<C: Context>(
    modules: Vec<&dyn ModuleInfo<Context = C>>,
) -> Result<Vec<&dyn ModuleInfo<Context = C>>, anyhow::Error> {
    let mut module_visitor = ModuleVisitor::<C>::new();
    module_visitor.visit_modules(modules)?;
    Ok(module_visitor.sorted_modules)
}

/// Accepts Vec<> of tuples (&ModuleInfo, &TValue), and returns Vec<&TValue> sorted by mapped module dependencies
pub fn sort_values_by_modules_dependencies<C: Context, TValue>(
    module_value_tuples: Vec<(&dyn ModuleInfo<Context = C>, TValue)>,
) -> Result<Vec<TValue>, anyhow::Error>
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
        let prev_entry = value_map.insert(module.0.address(), module.1);
        anyhow::ensure!(prev_entry.is_none(), "Duplicate module address! Only one instance of each module is allowed in a given runtime. Module with address {} is duplicated", module.0.address());
    }

    let mut sorted_values = Vec::new();
    for module in sorted_modules {
        sorted_values.push(value_map.get(module.address()).unwrap().clone());
    }

    Ok(sorted_values)
}

/// This trait is implemented by types that can be used as arguments in the sov-cli wallet.
/// The recommended way to implement this trait is using the provided derive macro (`#[derive(CliWalletArg)]`).
/// Currently, this trait is a thin wrapper around [`clap::Parser`]
#[cfg(feature = "native")]
pub trait CliWalletArg: From<Self::CliStringRepr> {
    /// The type that is used to represent this type in the CLI. Typically,
    /// this type implements the clap::Subcommand trait.
    type CliStringRepr;
}

/// A trait that needs to be implemented for a *runtime* to be used with the CLI wallet
#[cfg(feature = "native")]
pub trait CliWallet: sov_modules_core::DispatchCall {
    /// The type that is used to represent this type in the CLI. Typically,
    /// this type implements the clap::Subcommand trait. This type is generic to
    /// allow for different representations of the same type in the interface; a
    /// typical end-usage will impl traits only in the case where `CliStringRepr<T>: Into::RuntimeCall`
    type CliStringRepr<T>;
}
