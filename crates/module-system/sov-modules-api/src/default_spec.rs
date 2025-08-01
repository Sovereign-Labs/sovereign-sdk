use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::da::DaSpec;
#[cfg(feature = "native")]
use sov_rollup_interface::execution_mode::{Native, WitnessGeneration};
use sov_rollup_interface::zk::{CryptoSpec, ZkVerifier, Zkvm};
use sov_state::DefaultStorageSpec;

use crate::higher_kinded_types::{Generic, HigherKindedHelper};
use crate::{Address, GasUnit, Spec};

/// A default implementation of the [`Spec`] trait. Used for testing but can also be a good
/// starting point for implementing a custom rollup.
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(
    Default,
    serde::Serialize,
    serde::Deserialize,
    BorshDeserialize,
    BorshSerialize,
    schemars::JsonSchema,
)]
#[serde(bound = "")]
pub struct DefaultSpec<Da, InnerZkvm, OuterZkvm, Mode>(
    std::marker::PhantomData<(Da, InnerZkvm, OuterZkvm, Mode)>,
);

impl<Da: DaSpec, InnerZkvm: Zkvm, OuterZkvm: Zkvm, M> Generic
    for DefaultSpec<Da, InnerZkvm, OuterZkvm, M>
{
    type With<K> = DefaultSpec<Da, InnerZkvm, OuterZkvm, K>;
}

impl<Da: DaSpec, InnerZkvm: Zkvm, OuterZkvm: Zkvm, M> HigherKindedHelper
    for DefaultSpec<Da, InnerZkvm, OuterZkvm, M>
{
    type Inner = M;
}

mod default_impls {
    use sov_rollup_interface::execution_mode::ExecutionMode;

    use super::DefaultSpec;

    impl<Da, InnerZkvm, OuterZkvm, Mode: ExecutionMode> Clone
        for DefaultSpec<Da, InnerZkvm, OuterZkvm, Mode>
    {
        fn clone(&self) -> Self {
            Self(std::marker::PhantomData)
        }
    }

    impl<Da, InnerZkvm, OuterZkvm, Mode: ExecutionMode> PartialEq<Self>
        for DefaultSpec<Da, InnerZkvm, OuterZkvm, Mode>
    {
        fn eq(&self, _other: &Self) -> bool {
            true
        }
    }

    impl<Da, InnerZkvm, OuterZkvm, Mode: ExecutionMode> Eq
        for DefaultSpec<Da, InnerZkvm, OuterZkvm, Mode>
    {
    }

    impl<Da, InnerZkvm, OuterZkvm, Mode: ExecutionMode> core::fmt::Debug
        for DefaultSpec<Da, InnerZkvm, OuterZkvm, Mode>
    {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(
                f,
                "DefaultSpec<{}>",
                std::any::type_name::<(Da, InnerZkvm, OuterZkvm, Mode)>()
            )
        }
    }
}

#[cfg(feature = "native")]
impl<Da: DaSpec, InnerZkvm: Zkvm, OuterZkvm: Zkvm> Spec
    for DefaultSpec<Da, InnerZkvm, OuterZkvm, WitnessGeneration>
where
    <InnerZkvm::Verifier as ZkVerifier>::CryptoSpec: crate::CryptoSpecExt,
{
    type Da = Da;
    type Address = Address;
    type Gas = GasUnit<2>;

    type Storage =
        sov_state::ProverStorage<DefaultStorageSpec<<Self::CryptoSpec as CryptoSpec>::Hasher>>;

    type InnerZkvm = InnerZkvm;
    type OuterZkvm = OuterZkvm;

    type CryptoSpec = <InnerZkvm::Verifier as ZkVerifier>::CryptoSpec;
}

#[cfg(feature = "native")]
impl<Da: DaSpec, InnerZkvm: Zkvm, OuterZkvm: Zkvm> Spec
    for DefaultSpec<Da, InnerZkvm, OuterZkvm, Native>
where
    <InnerZkvm::Verifier as ZkVerifier>::CryptoSpec: crate::CryptoSpecExt,
{
    type Da = Da;
    type Address = Address;
    type Gas = GasUnit<2>;

    // This TODO is for performance enhancement, not a security concern.
    // TODO: Replace ProverStorage with an optimized impl!
    type Storage =
        sov_state::ProverStorage<DefaultStorageSpec<<Self::CryptoSpec as CryptoSpec>::Hasher>>;

    type InnerZkvm = InnerZkvm;
    type OuterZkvm = OuterZkvm;

    type CryptoSpec = <InnerZkvm::Verifier as ZkVerifier>::CryptoSpec;
}

#[cfg(any(not(feature = "native"), feature = "test-utils"))]
impl<Da: DaSpec, InnerZkvm: Zkvm, OuterZkvm: Zkvm> Spec
    for DefaultSpec<Da, InnerZkvm, OuterZkvm, crate::execution_mode::Zk>
where
    <InnerZkvm::Verifier as ZkVerifier>::CryptoSpec: crate::CryptoSpecExt,
{
    type Da = Da;
    type Address = Address;
    type Gas = GasUnit<2>;

    type Storage =
        sov_state::ZkStorage<DefaultStorageSpec<<Self::CryptoSpec as CryptoSpec>::Hasher>>;

    type InnerZkvm = InnerZkvm;
    type OuterZkvm = OuterZkvm;

    type CryptoSpec = <InnerZkvm::Verifier as ZkVerifier>::CryptoSpec;
}
