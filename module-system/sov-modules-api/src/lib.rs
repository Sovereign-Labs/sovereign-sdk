#![doc = include_str!("../README.md")]

mod bech32;
pub mod capabilities;
#[cfg(feature = "native")]
pub mod cli;
pub mod default_context;
pub mod default_signature;
mod dispatch;
mod encode;
mod error;
pub mod hooks;

#[cfg(feature = "macros")]
mod reexport_macros;
#[cfg(feature = "macros")]
pub use reexport_macros::*;

mod prefix;
mod response;
mod serde_address;
#[cfg(test)]
mod tests;
pub mod transaction;
#[cfg(feature = "native")]
pub mod utils;

#[cfg(feature = "macros")]
extern crate sov_modules_macros;

use core::fmt::{self, Debug, Display};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "native")]
pub use clap;
use digest::typenum::U32;
use digest::Digest;
#[cfg(feature = "native")]
pub use dispatch::CliWallet;
pub use dispatch::{DispatchCall, EncodeCall, Genesis};
pub use error::Error;
pub use prefix::Prefix;
pub use response::CallResponse;
#[cfg(feature = "native")]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
pub use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
pub use sov_rollup_interface::services::da::SlotData;
pub use sov_rollup_interface::stf::Event;
pub use sov_rollup_interface::zk::{
    StateTransition, ValidityCondition, ValidityConditionChecker, Zkvm,
};
pub use sov_rollup_interface::{digest, BasicAddress, RollupAddress};
use sov_state::{Storage, Witness, WorkingSet};
use thiserror::Error;

pub use crate::bech32::AddressBech32;

pub mod optimistic {
    pub use sov_rollup_interface::optimistic::{Attestation, ProofOfBond};
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl BasicAddress for Address {}
impl RollupAddress for Address {}

#[cfg_attr(feature = "native", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(PartialEq, Clone, Copy, Eq, borsh::BorshDeserialize, borsh::BorshSerialize, Hash)]
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

impl FromStr for Address {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AddressBech32::from_str(s)
            .map_err(|e| anyhow::anyhow!(e))
            .map(|addr_bech32| addr_bech32.into())
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

    fn verify(&self, pub_key: &Self::PublicKey, msg: &[u8]) -> Result<(), SigVerificationError>;
}

/// A type that can't be instantiated.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "native", derive(schemars::JsonSchema))]
pub enum NonInstantiable {}

/// PublicKey used in the Module System.
pub trait PublicKey {
    fn to_address<A: RollupAddress>(&self) -> A;
}

/// A PrivateKey used in the Module System.
#[cfg(feature = "native")]
pub trait PrivateKey {
    type PublicKey: PublicKey;
    type Signature: Signature<PublicKey = Self::PublicKey>;
    fn generate() -> Self;
    fn pub_key(&self) -> Self::PublicKey;
    fn sign(&self, msg: &[u8]) -> Self::Signature;
    fn to_address<A: RollupAddress>(&self) -> A {
        self.pub_key().to_address::<A>()
    }
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
    type Address: RollupAddress
        + BorshSerialize
        + BorshDeserialize
        + Sync
        // Do we always need this, even when the module does not have a JSON
        // Schema? That feels a bit wrong.
        + ::schemars::JsonSchema
        + Into<AddressBech32>
        + From<AddressBech32>
        + FromStr<Err = anyhow::Error>;

    /// The Address type used on the rollup. Typically calculated as the hash of a public key.
    #[cfg(not(feature = "native"))]
    type Address: RollupAddress + BorshSerialize + BorshDeserialize;

    /// Authenticated state storage used by the rollup. Typically some variant of a merkle-patricia trie.
    type Storage: Storage + Clone + Send + Sync;

    /// The public key used for digital signatures
    #[cfg(feature = "native")]
    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Hash
        + Clone
        + Debug
        + PublicKey
        + Serialize
        + for<'a> Deserialize<'a>
        + ::schemars::JsonSchema
        + Send
        + Sync
        + FromStr<Err = anyhow::Error>;

    /// The public key used for digital signatures
    #[cfg(feature = "native")]
    type PrivateKey: Debug
        + Send
        + Sync
        + for<'a> TryFrom<&'a [u8], Error = anyhow::Error>
        + Serialize
        + DeserializeOwned
        + PrivateKey<PublicKey = Self::PublicKey, Signature = Self::Signature>;

    #[cfg(not(feature = "native"))]
    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Hash
        + Clone
        + Debug
        + Send
        + Sync
        + PublicKey;

    /// The hasher preferred by the rollup, such as Sha256 or Poseidon.
    type Hasher: Digest<OutputSize = U32>;

    /// The digital signature scheme used by the rollup
    #[cfg(feature = "native")]
    type Signature: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Serialize
        + for<'a> Deserialize<'a>
        + schemars::JsonSchema
        + Eq
        + Clone
        + Debug
        + Send
        + Sync
        + FromStr<Err = anyhow::Error>
        + Signature<PublicKey = Self::PublicKey>;

    /// The digital signature scheme used by the rollup
    #[cfg(not(feature = "native"))]
    type Signature: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + Signature<PublicKey = Self::PublicKey>
        + Send
        + Sync;

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
pub trait Context: Spec + Clone + Debug + PartialEq + 'static {
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
pub trait Module: Default {
    /// Execution context.
    type Context: Context;

    /// Configuration for the genesis method.
    type Config;

    /// Module defined argument to the call method.
    type CallMessage: Debug + BorshSerialize + BorshDeserialize;

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

    /// Returns the prefix of the module.
    fn prefix(&self) -> Prefix;

    /// Returns addresses of all the other modules this module is dependent on
    fn dependencies(&self) -> Vec<&<Self::Context as Spec>::Address>;
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
