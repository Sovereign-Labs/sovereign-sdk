//! Defines namespaces that are used to partition the state of the rollup.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
/// The namespaces used in the rollup. Related to the db's namespaces.
pub enum Namespace {
    /// The user namespace. Used by the User modules and is synchronized with the visible height.
    User,
    /// The kernel namespace. Used by the Kernel modules and is synchronized with the true height.
    Kernel,
    /// The accessory namespace. Values in this namespace are writeable but not readable inside the state transition
    /// function. They are used to provide auxiliary data via API.
    Accessory,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    UniversalWallet,
)]
#[serde(rename_all = "snake_case")]
/// Namespaces which can be merkle proven.
pub enum ProvableNamespace {
    /// The user namespace.
    User,
    /// The kernel namespace.
    Kernel,
}

/// Converts a type into a runtime namespace.
pub trait CompileTimeNamespace: core::fmt::Debug + Send + Sync + 'static {
    /// The runtime namespace variant associated with the type.
    const NAMESPACE: Namespace;
}

/// Converts a type into a Provable Namespace at compile time.
pub trait ProvableCompileTimeNamespace: CompileTimeNamespace {
    /// The runtime namespace variant associated with the type.
    const PROVABLE_NAMESPACE: ProvableNamespace;
}

/// A type-level representation of the user namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct User;

impl CompileTimeNamespace for User {
    const NAMESPACE: Namespace = Namespace::User;
}

impl ProvableCompileTimeNamespace for User {
    const PROVABLE_NAMESPACE: ProvableNamespace = ProvableNamespace::User;
}
/// A type-level representation of the kernel namespace
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Kernel;

impl CompileTimeNamespace for Kernel {
    const NAMESPACE: Namespace = Namespace::Kernel;
}

impl ProvableCompileTimeNamespace for Kernel {
    const PROVABLE_NAMESPACE: ProvableNamespace = ProvableNamespace::Kernel;
}

/// A type-level representation of the accessory namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Accessory;

impl CompileTimeNamespace for Accessory {
    const NAMESPACE: Namespace = Namespace::Accessory;
}
