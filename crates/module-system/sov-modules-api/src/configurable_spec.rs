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

#[cfg(feature = "native")]
type DefaultStorage<StorageSpec> = sov_state::ProverStorage<StorageSpec>;

#[cfg(not(feature = "native"))]
type DefaultStorage<StorageSpec> = sov_state::ZkStorage<StorageSpec>;

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
    Storage = DefaultStorage<DefaultStorageSpec<<CryptoSpec as CryptoSpecTrait>::Hasher>>,
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

#[cfg(feature = "native")]
impl<
        Da: DaSpec,
        InnerZkvm: Zkvm,
        OuterZkvm: Zkvm,
        CryptoSpec: CryptoSpecExt,
        Address: BasicAddress,
        Storage: sov_state::Storage + sov_state::NativeStorage + Send + Sync + 'static,
    > Spec
    for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, WitnessGeneration, CryptoSpec, Storage>
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

#[cfg(feature = "native")]
impl<
        Da: DaSpec,
        InnerZkvm: Zkvm,
        OuterZkvm: Zkvm,
        CryptoSpec: CryptoSpecExt,
        Address: BasicAddress,
        Storage: sov_state::Storage + sov_state::NativeStorage + Send + Sync + 'static,
    > Spec for ConfigurableSpec<Da, InnerZkvm, OuterZkvm, Address, Native, CryptoSpec, Storage>
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
