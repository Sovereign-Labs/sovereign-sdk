use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::crypto::CredentialId;
use sov_rollup_interface::da::DaSpec;
#[cfg(feature = "native")]
use sov_rollup_interface::execution_mode::{Native, WitnessGeneration};
use sov_rollup_interface::zk::{CryptoSpec as CryptoSpecTrait, ZkVerifier, Zkvm};
use sov_rollup_interface::BasicAddress;
use sov_state::DefaultStorageSpec;

use crate::higher_kinded_types::{Generic, HigherKindedHelper};
use crate::{CryptoSpecExt, GasUnit, Spec};

// Internal helper that lets the `Storage =` default depend on both `Da` and
// `StorageSpec`, even in the Zk variant where the resulting storage type
// happens not to mention `Da`. A direct `type` alias would fail E0091 in the
// Zk variant because `Da` would be unused on the right-hand side.
#[doc(hidden)]
pub struct DefaultStorageMarker<Da, StorageSpec>(PhantomData<(Da, StorageSpec)>);

#[doc(hidden)]
pub trait DefaultStorageOf {
    type Storage;
}

#[cfg(feature = "native")]
impl<Da: DaSpec, StorageSpec: sov_state::MerkleProofSpec> DefaultStorageOf
    for DefaultStorageMarker<Da, StorageSpec>
{
    type Storage = sov_state::nomt::prover_storage::NomtProverStorage<StorageSpec, Da::SlotHash>;
}

#[cfg(not(feature = "native"))]
impl<Da, StorageSpec: sov_state::MerkleProofSpec> DefaultStorageOf
    for DefaultStorageMarker<Da, StorageSpec>
{
    type Storage = sov_state::nomt::zk_storage::NomtVerifierStorage<StorageSpec>;
}

type DefaultStorage<Da, StorageSpec> =
    <DefaultStorageMarker<Da, StorageSpec> as DefaultStorageOf>::Storage;

/// A default implementation of the [`Spec`] trait. Used for testing but can also be a good
/// starting point for implementing a custom rollup.
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(
    serde::Serialize, serde::Deserialize, BorshDeserialize, BorshSerialize, schemars::JsonSchema,
)]
#[serde(bound = "")]
#[schemars(
    rename = "ConfigurableSpec",
    bound = "Da: ::schemars::JsonSchema, InnerZkvm: ::schemars::JsonSchema, OuterZkvm: ::schemars::JsonSchema, CryptoSpec: ::schemars::JsonSchema, Address: ::schemars::JsonSchema, Mode: ::schemars::JsonSchema"
)]
pub struct ConfigurableSpec<
    Da,
    InnerZkvm,
    OuterZkvm,
    Address,
    Mode,
    CryptoSpec = <<InnerZkvm as Zkvm>::Verifier as ZkVerifier>::CryptoSpec,
    Storage = DefaultStorage<Da, DefaultStorageSpec<<CryptoSpec as CryptoSpecTrait>::Hasher>>,
>(PhantomData<(Da, InnerZkvm, OuterZkvm, CryptoSpec, Address, Mode, Storage)>);

impl<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage> Default
    for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage>
{
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<Da: DaSpec, InnerZkvm: Zkvm, OuterZkvm: Zkvm, Address, Mode, CryptoSpec, Storage> Generic
    for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage>
{
    type With<K> = ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, K, CryptoSpec, Storage>;
}

impl<Da: DaSpec, InnerZkvm: Zkvm, OuterZkvm: Zkvm, Address, Mode, CryptoSpec, Storage>
    HigherKindedHelper
    for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage>
{
    type Inner = Mode;
}

mod default_impls {

    use std::marker::PhantomData;

    use super::ConfigurableSpec;

    impl<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage> Clone
        for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage>
    {
        fn clone(&self) -> Self {
            Self(PhantomData)
        }
    }

    impl<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage> PartialEq<Self>
        for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage>
    {
        fn eq(&self, _other: &Self) -> bool {
            true
        }
    }

    impl<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage> Eq
        for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage>
    {
    }

    impl<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage> core::fmt::Debug
        for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, Mode, CryptoSpec, Storage>
    {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(
                f,
                "ConfigurableSpec<{}>",
                std::any::type_name::<(Da, InnerZkvm, OuterZkvm, Mode, Storage)>()
            )
        }
    }
}

// Native and WitnessGeneration share an identical `Spec` impl; expand it for each via macro
// to avoid coherence conflicts with the Zk impl below when `test-utils` is enabled.
#[cfg(feature = "native")]
macro_rules! impl_configurable_spec_native {
    ($mode:ty) => {
        impl<
                Da: DaSpec,
                InnerZkvm: Zkvm,
                OuterZkvm: Zkvm,
                CryptoSpec: CryptoSpecExt,
                Address: BasicAddress,
                Storage: sov_state::Storage + sov_state::NativeStorage + Send + Sync + 'static,
            > Spec
            for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, $mode, CryptoSpec, Storage>
        where
            Address: From<CredentialId>,
        {
            type Da = Da;
            type Gas = GasUnit<2>;
            type Address = Address;

            type Storage = Storage;

            type InnerZkvm = InnerZkvm;
            type OuterZkvm = OuterZkvm;

            type CryptoSpec = CryptoSpec;
        }
    };
}

#[cfg(feature = "native")]
impl_configurable_spec_native!(Native);
#[cfg(feature = "native")]
impl_configurable_spec_native!(WitnessGeneration);

#[cfg(not(feature = "native"))]
impl<
        Da: DaSpec,
        InnerZkvm: Zkvm,
        OuterZkvm: Zkvm,
        CryptoSpec: CryptoSpecExt,
        Address: BasicAddress,
        Storage: sov_state::Storage + Send + Sync + 'static,
    > Spec
    for ConfigurableSpec<
        Da,
        InnerZkvm,
        OuterZkvm,
        Address,
        crate::execution_mode::Zk,
        CryptoSpec,
        Storage,
    >
where
    Address: From<CredentialId>,
{
    type Da = Da;
    type Address = Address;
    type Gas = GasUnit<2>;

    type Storage = Storage;

    type InnerZkvm = InnerZkvm;
    type OuterZkvm = OuterZkvm;

    type CryptoSpec = CryptoSpec;
}
