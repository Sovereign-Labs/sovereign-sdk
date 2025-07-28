//! This file defines all the possible ways to access the state of the rollup for the
//! accessors defined in this module.

use std::convert::Infallible;

#[cfg(feature = "native")]
use sov_state::Accessory;
use sov_state::{
    Kernel, SlotKey, SlotValue, StateCodec, StateItemCodec, StateItemDecoder, Storage, User,
};

use super::genesis::GenesisStateAccessor;
use super::StateProvider;
use crate::state::traits::{
    get_inner, AccessoryStateWriter, ProvableStateReader, ProvableStateWriter,
};
#[cfg(feature = "native")]
use crate::AccessoryStateCheckpoint;
use crate::{
    AccessoryDelta, AccessoryStateReader, PreExecWorkingSet, Spec, StateCheckpoint, StateReader,
    StateWriter, TxScratchpad, WorkingSet,
};

macro_rules! inner_impl_charge_gas_state_infallible_reader {
    ($namespace:ty) => {
        type Error = Infallible;

        fn get(&mut self, key: &SlotKey) -> Result<Option<SlotValue>, Infallible> {
            let val = get_inner(
                self,
                <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                key,
            )
            .expect("We should never fail to charge gas for infallible accessor. This is a bug!");

            Ok(val)
        }

        fn get_decoded<V, Codec>(
            &mut self,
            storage_key: &SlotKey,
            codec: &Codec,
        ) -> Result<Option<V>, Infallible>
        where
            Codec: StateCodec,
            Codec::ValueCodec: StateItemCodec<V>,
        {
            let storage_value = <Self as StateReader<$namespace>>::get(self, storage_key)?;
            Ok(storage_value
                .map(|storage_value| codec.value_codec().decode_unwrap(storage_value.value())))
        }
    };
}

macro_rules! inner_impl_charge_gas_infallible_state_writer {
    ($namespace:ty) => {
        type Error = Infallible;

        fn set(&mut self, key: &SlotKey, value: SlotValue) -> Result<(), Infallible> {
            crate::state::traits::set_inner(
                self,
                <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                key,
                value,
            )
            .expect("We should never fail to charge gas for infallible accessor. This is a bug!");

            Ok(())
        }

        fn delete(&mut self, key: &SlotKey) -> Result<(), Infallible> {
            crate::state::traits::delete_inner(
                self,
                <$namespace as sov_state::CompileTimeNamespace>::NAMESPACE,
                key,
            )
            .expect("We should never fail to charge gas for infallible accessor. This is a bug!");

            Ok(())
        }
    };
}

#[cfg(feature = "native")]
mod http_api {
    use super::*;
    use crate::state::accessors::http_api::ApiStateAccessor;
    use crate::{AccessoryStateReader, AccessoryStateWriter, Spec, StateReader, StateWriter};

    impl<S: Spec> AccessoryStateReader for ApiStateAccessor<S> {}
    impl<S: Spec> AccessoryStateWriter for ApiStateAccessor<S> {}

    impl<S: Spec> StateReader<User> for ApiStateAccessor<S> {
        inner_impl_charge_gas_state_infallible_reader!(User);
    }
    impl<S: Spec> StateWriter<User> for ApiStateAccessor<S> {
        inner_impl_charge_gas_infallible_state_writer!(User);
    }

    impl<S: Spec> StateReader<Kernel> for ApiStateAccessor<S> {
        inner_impl_charge_gas_state_infallible_reader!(Kernel);
    }
    impl<S: Spec> StateWriter<Kernel> for ApiStateAccessor<S> {
        inner_impl_charge_gas_infallible_state_writer!(Kernel);
    }
}

impl<S: Storage> AccessoryStateReader for AccessoryDelta<S> {}
impl<S: Storage> AccessoryStateWriter for AccessoryDelta<S> {}

impl<S: Spec> StateReader<User> for GenesisStateAccessor<'_, S> {
    inner_impl_charge_gas_state_infallible_reader!(User);
}
impl<S: Spec> StateWriter<User> for GenesisStateAccessor<'_, S> {
    inner_impl_charge_gas_infallible_state_writer!(User);
}
impl<S: Spec> StateReader<Kernel> for GenesisStateAccessor<'_, S> {
    inner_impl_charge_gas_state_infallible_reader!(Kernel);
}
impl<S: Spec> StateWriter<Kernel> for GenesisStateAccessor<'_, S> {
    inner_impl_charge_gas_infallible_state_writer!(Kernel);
}

impl<S: Spec> AccessoryStateWriter for GenesisStateAccessor<'_, S> {}

impl<S: Spec> StateReader<User> for StateCheckpoint<S> {
    inner_impl_charge_gas_state_infallible_reader!(User);
}
impl<S: Spec> StateWriter<User> for StateCheckpoint<S> {
    inner_impl_charge_gas_infallible_state_writer!(User);
}

impl<S: Spec> StateReader<Kernel> for StateCheckpoint<S> {
    inner_impl_charge_gas_state_infallible_reader!(Kernel);
}
impl<S: Spec> StateWriter<Kernel> for StateCheckpoint<S> {
    inner_impl_charge_gas_infallible_state_writer!(Kernel);
}

impl<S: Spec> AccessoryStateWriter for StateCheckpoint<S> {}

impl<S, I> StateReader<User> for TxScratchpad<S, I>
where
    S: Spec,
    I: StateProvider<S>,
{
    inner_impl_charge_gas_state_infallible_reader!(User);
}
impl<S, I> StateReader<Kernel> for TxScratchpad<S, I>
where
    S: Spec,
    I: StateProvider<S>,
{
    inner_impl_charge_gas_state_infallible_reader!(Kernel);
}

impl<S: Spec, I: StateProvider<S>> StateWriter<User> for TxScratchpad<S, I> {
    inner_impl_charge_gas_infallible_state_writer!(User);
}

impl<S: Spec, I: StateProvider<S>> ProvableStateReader<User> for PreExecWorkingSet<S, I> {}
impl<S: Spec, I: StateProvider<S>> ProvableStateWriter<User> for PreExecWorkingSet<S, I> {}

impl<S: Spec, I: StateProvider<S>> ProvableStateReader<User> for WorkingSet<S, I> {}
impl<S: Spec, I: StateProvider<S>> ProvableStateReader<Kernel> for WorkingSet<S, I> {}
impl<S: Spec, I: StateProvider<S>> ProvableStateWriter<User> for WorkingSet<S, I> {}
impl<S: Spec, I: StateProvider<S>> ProvableStateWriter<Kernel> for WorkingSet<S, I> {}

impl<S: Spec, I: StateProvider<S>> AccessoryStateWriter for WorkingSet<S, I> {}

#[cfg(feature = "test-utils")]
impl<S: Spec, I: StateProvider<S>> AccessoryStateReader for WorkingSet<S, I> {}

#[cfg(feature = "native")]
impl<S: Spec> StateReader<Accessory> for AccessoryStateCheckpoint<'_, S> {
    type Error = Infallible;
    fn get(&mut self, key: &SlotKey) -> Result<Option<SlotValue>, Self::Error> {
        Ok(self.checkpoint.delta.get(
            <Accessory as sov_state::CompileTimeNamespace>::NAMESPACE,
            key,
        ))
    }

    /// Get a decoded value from the storage.
    fn get_decoded<V, Codec>(
        &mut self,
        storage_key: &SlotKey,
        codec: &Codec,
    ) -> Result<Option<V>, Self::Error>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateItemCodec<V>,
    {
        let storage_value = <Self as StateReader<Accessory>>::get(self, storage_key)?;

        Ok(storage_value
            .map(|storage_value| codec.value_codec().decode_unwrap(storage_value.value())))
    }
}

#[cfg(feature = "native")]
impl<S: Spec> AccessoryStateWriter for AccessoryStateCheckpoint<'_, S> {}

pub mod kernel_state {
    use std::convert::Infallible;

    use sov_state::{
        Kernel, SlotKey, SlotValue, StateCodec, StateItemCodec, StateItemDecoder, User,
    };

    use super::get_inner;
    use crate::state::accessors::BootstrapWorkingSet;
    use crate::{KernelStateAccessor, Spec, StateReader, StateWriter};

    impl<S: Spec> StateReader<Kernel> for BootstrapWorkingSet<'_, S> {
        inner_impl_charge_gas_state_infallible_reader!(Kernel);
    }
    impl<S: Spec> StateReader<User> for BootstrapWorkingSet<'_, S> {
        inner_impl_charge_gas_state_infallible_reader!(User);
    }

    impl<S: Spec> StateWriter<Kernel> for BootstrapWorkingSet<'_, S> {
        inner_impl_charge_gas_infallible_state_writer!(Kernel);
    }
    impl<S: Spec> StateWriter<User> for BootstrapWorkingSet<'_, S> {
        inner_impl_charge_gas_infallible_state_writer!(User);
    }

    impl<S: Spec> StateReader<Kernel> for KernelStateAccessor<'_, S> {
        inner_impl_charge_gas_state_infallible_reader!(Kernel);
    }
    impl<S: Spec> StateReader<User> for KernelStateAccessor<'_, S> {
        inner_impl_charge_gas_state_infallible_reader!(User);
    }

    impl<S: Spec> StateWriter<Kernel> for KernelStateAccessor<'_, S> {
        inner_impl_charge_gas_infallible_state_writer!(Kernel);
    }
    impl<S: Spec> StateWriter<User> for KernelStateAccessor<'_, S> {
        inner_impl_charge_gas_infallible_state_writer!(User);
    }
}
