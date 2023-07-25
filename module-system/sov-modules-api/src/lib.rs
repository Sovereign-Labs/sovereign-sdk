#![feature(associated_type_defaults)]

mod bech32;
pub mod default_context;
pub mod default_signature;
mod dispatch;
mod encode;
mod error;
pub mod hooks;
mod prefix;
mod response;
mod serde_address;
#[cfg(test)]
mod tests;
pub mod transaction;

#[cfg(feature = "macros")]
extern crate sov_modules_macros;

#[cfg(feature = "macros")]
pub use sov_modules_macros::{
    DispatchCall, Genesis, MessageCodec, ModuleCallJsonSchema, ModuleInfo,
};

/// Procedural macros to assist with creating new modules.
#[cfg(feature = "macros")]
pub mod macros {
    pub use sov_modules_macros::{cli_parser, expose_rpc, rpc_gen, DefaultRuntime, MessageCodec};
}

use core::fmt::{self, Debug, Display};
use std::collections::{HashMap, HashSet};

use borsh::{BorshDeserialize, BorshSerialize};
pub use dispatch::{DispatchCall, Genesis};
pub use error::Error;
pub use prefix::Prefix;
pub use response::CallResponse;
use serde::{Deserialize, Serialize};
pub use sov_rollup_interface::crypto::SimpleHasher as Hasher;
pub use sov_rollup_interface::AddressTrait;
use sov_state::{Storage, Witness, WorkingSet};
use thiserror::Error;

pub use crate::bech32::AddressBech32;

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl std::hash::Hash for Address {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.addr.hash(state);
    }
}

impl AddressTrait for Address {}

#[cfg_attr(feature = "native", derive(schemars::JsonSchema))]
#[derive(PartialEq, Clone, Eq, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Address {
    addr: [u8; 32],
}

impl<'a> TryFrom<&'a [u8]> for Address {
    type Error = anyhow::Error;

    fn try_from(addr: &'a [u8]) -> Result<Self, Self::Error> {
        if addr.len() != 32 {
            anyhow::bail!("Address must be 32 bytes long");
        }
        let mut addr_bytes = [0u8; 32];
        addr_bytes.copy_from_slice(addr);
        Ok(Self { addr: addr_bytes })
    }
}

impl From<[u8; 32]> for Address {
    fn from(addr: [u8; 32]) -> Self {
        Self { addr }
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", AddressBech32::from(self))
    }
}

impl Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", AddressBech32::from(self))
    }
}

impl From<AddressBech32> for Address {
    fn from(addr: AddressBech32) -> Self {
        Self {
            addr: addr.to_byte_array(),
        }
    }
}

#[derive(Error, Debug)]
pub enum SigVerificationError {
    #[error("Bad signature {0}")]
    BadSignature(String),
}

/// Signature used in the Module System.
pub trait Signature {
    type PublicKey;

    fn verify(
        &self,
        pub_key: &Self::PublicKey,
        msg_hash: [u8; 32],
    ) -> Result<(), SigVerificationError>;
}

/// A type that can't be instantiated.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum NonInstantiable {}

/// PublicKey used in the Module System.
pub trait PublicKey {
    fn to_address<A: AddressTrait>(&self) -> A;
}

/// The `Spec` trait configures certain key primitives to be used by a by a particular instance of a rollup.
/// `Spec` is almost always implemented on a Context object; since all Modules are generic
/// over a Context, rollup developers can easily optimize their code for different environments
/// by simply swapping out the Context (and by extension, the Spec).
///
/// For example, a rollup running in a STARK-based zkvm like Risc0 might pick Sha256 or Poseidon as its preferred hasher,
/// while a rollup running in an elliptic-curve based SNARK such as `Placeholder` from the =nil; foundation might
/// prefer a Pedersen hash. By using a generic Context and Spec, a rollup developer can trivially customize their
/// code for either (or both!) of these environments without touching their module implementations.
pub trait Spec {
    /// The Address type used on the rollup. Typically calculated as the hash of a public key.
    #[cfg(feature = "native")]
    type Address: AddressTrait
        + BorshSerialize
        + BorshDeserialize
        // Do we always need this, even when the module does not have a JSON
        // Schema? That feels a bit wrong.
        + ::schemars::JsonSchema
        + Into<AddressBech32>
        + From<AddressBech32>;

    /// The Address type used on the rollup. Typically calculated as the hash of a public key.
    #[cfg(not(feature = "native"))]
    type Address: AddressTrait + BorshSerialize + BorshDeserialize;

    /// Authenticated state storage used by the rollup. Typically some variant of a merkle-patricia trie.
    type Storage: Storage + Clone + Send + Sync;

    /// The public key used for digital signatures
    #[cfg(feature = "native")]
    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + PublicKey
        + Serialize
        + for<'a> Deserialize<'a>
        + ::schemars::JsonSchema
        + Send
        + Sync;

    #[cfg(not(feature = "native"))]
    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + Send
        + Sync
        + PublicKey;

    /// The hasher preferred by the rollup, such as Sha256 or Poseidon.
    type Hasher: Hasher;

    /// The digital signature scheme used by the rollup
    #[cfg(feature = "native")]
    type Signature: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + schemars::JsonSchema
        + Eq
        + Clone
        + Debug
        + Signature<PublicKey = Self::PublicKey>;

    #[cfg(not(feature = "native"))]
    type Signature: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + Signature<PublicKey = Self::PublicKey>;

    /// A structure containing the non-deterministic inputs from the prover to the zk-circuit
    type Witness: Witness;
}

/// A context contains information which is passed to modules during
/// transaction execution. Currently, context includes the sender of the transaction
/// as recovered from its signature.
///
/// Context objects also implement the [`Spec`] trait, which specifies the types to be used in this
/// instance of the state transition function. By making modules generic over a `Context`, developers
/// can easily update their cryptography to conform to the needs of different zk-proof systems.
pub trait Context: Spec + Clone + Debug + PartialEq {
    /// Sender of the transaction.
    fn sender(&self) -> &Self::Address;

    /// Constructor for the Context.
    fn new(sender: Self::Address) -> Self;
}

impl<T> Genesis for T
where
    T: Module,
{
    type Context = <Self as Module>::Context;

    type Config = <Self as Module>::Config;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<<<Self as Genesis>::Context as Spec>::Storage>,
    ) -> Result<(), Error> {
        <Self as Module>::genesis(self, config, working_set)
    }
}

/// All the methods have a default implementation that can't be invoked (because they take `NonInstantiable` parameter).
/// This allows developers to override only some of the methods in their implementation and safely ignore the others.
pub trait Module {
    /// Execution context.
    type Context: Context;

    /// Configuration for the genesis method.
    type Config;

    /// Module defined argument to the call method.
    type CallMessage: Debug + BorshSerialize + BorshDeserialize = NonInstantiable;

    /// Genesis is called when a rollup is deployed and can be used to set initial state values in the module.
    fn genesis(
        &self,
        _config: &Self::Config,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Call allows interaction with the module and invokes state changes.
    /// It takes a module defined type and a context as parameters.
    fn call(
        &self,
        _message: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> Result<CallResponse, Error> {
        unreachable!()
    }
}

/// A [`Module`] that has a well-defined and known [JSON
/// Schema](https://json-schema.org/) for its [`Module::CallMessage`].
///
/// This trait is intended to support code generation tools, CLIs, and
/// documentation. You can derive it with `#[derive(ModuleCallJsonSchema)]`, or
/// implement it manually if your use case demands more control over the JSON
/// Schema generation.
pub trait ModuleCallJsonSchema: Module {
    /// Returns the JSON schema for [`Module::CallMessage`].
    fn json_schema() -> String;
}

/// Every module has to implement this trait.
pub trait ModuleInfo {
    type Context: Context;

    /// Returns address of the module.
    fn address(&self) -> &<Self::Context as Spec>::Address;

    /// Returns addresses of all the other modules this module is dependent on
    fn dependencies(&self) -> Vec<&<Self::Context as Spec>::Address>;
}

/// A StateTransitionRunner needs to implement this if
/// the RPC service is needed
pub trait RpcRunner {
    type Context: Context;
    fn get_storage(&self) -> <Self::Context as Spec>::Storage;
}

/// A module, with an extra item attached to it
pub type ModuleWithItem<'a, C, T> = (&'a dyn ModuleInfo<Context = C>, T);

/// Sorts the provided slice of (module, item) tuples in place, ordering the result by the dependencies of the modules.
/// Note that the module dependency graphonly defines a partial ordering,
/// so the exact results will depend on the ordering of the input.
pub fn sort_modules_by_dependencies<'a, C: Context, T>(
    modules: &mut Vec<ModuleWithItem<'a, C, T>>,
) {
    /// For each address in the list, recursively visit all of its dependencies and place them into the sorted array.
    fn visit_dependencies<'a, 'sort, 'visit, C: Context, T>(
        dependencies: Vec<&'a C::Address>,
        modules_to_visit: &'visit mut HashMap<
            &'sort <C as Spec>::Address,
            ModuleWithItem<'a, C, T>,
        >,
        all_module_addresses: &'sort HashSet<&'a C::Address>,
        sorted_modules: &mut Vec<ModuleWithItem<'a, C, T>>,
    ) {
        for dep in dependencies {
            // Sanity check that the dependency address is valid
            if !all_module_addresses.contains(dep) {
                panic!("The module with address {} is in your dependency tree but was not found in the runtime and could not be initialized. Make sure that all modules are declared in your runtime.", dep)
            }
            if let Some(module_with_item) = modules_to_visit.remove(dep) {
                let transitive_dep_addrs = module_with_item.0.dependencies();
                visit_dependencies(
                    transitive_dep_addrs,
                    modules_to_visit,
                    all_module_addresses,
                    sorted_modules,
                );
                sorted_modules.push(module_with_item)
            }
        }
    }
    let all_module_addresses = modules.iter().map(|m| m.0.address()).collect();
    let mut sorted_modules: Vec<ModuleWithItem<'a, C, T>> = Vec::with_capacity(modules.len());
    let mut modules_to_visit: HashMap<&C::Address, ModuleWithItem<'a, C, T>> =
        std::mem::take(modules)
            .into_iter()
            .map(|m| (m.0.address(), m))
            .collect();

    while !modules_to_visit.is_empty() {
        // Pick the next available module as a starting point.
        let (current_address, current_deps) = {
            let (addr, current_module_ref) = modules_to_visit.iter().next().unwrap();
            let current_deps = current_module_ref.0.dependencies();
            (addr.clone(), current_deps)
        };

        // Recursively place all of its dependencies in the sorted array.
        visit_dependencies(
            current_deps,
            &mut modules_to_visit,
            &all_module_addresses,
            &mut sorted_modules,
        );

        // Place the module in the sorted array.
        let current_module = modules_to_visit
            .remove(current_address)
            .expect("Dependency cycle detected! The module with address {} has itself as a transitive dependency.");
        sorted_modules.push(current_module);

        // Continue until there are no unvisited modules left.
    }

    *modules = sorted_modules;
}
