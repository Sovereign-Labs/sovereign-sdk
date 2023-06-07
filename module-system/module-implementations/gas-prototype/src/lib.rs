use sov_modules_api::Error;
use sov_modules_api::Hasher;
use sov_state::{GasUnit, StateValue, WorkingSet};
use std::marker::PhantomData;

pub struct SomeConfig<C: sov_modules_api::Context> {
    _p: PhantomData<C>,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct CallMessage<C: sov_modules_api::Context> {
    _p: PhantomData<C>,
}

pub struct GasConfig<GasUnit> {
    pub method_1_cost: GasUnit,
}

//#[derive(ModuleInfo, Clone)]
pub struct SomeModule<C: sov_modules_api::Context> {
    //#[address]
    pub(crate) address: C::Address,

    /// #[state]
    pub(crate) some_state_value: StateValue<u64>,

    //#[gas] Q should we hide it and gerenrate it?
    pub(crate) gas_config: GasConfig<C::GasUnit>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for SomeModule<C> {
    type Context = C;

    type Config = SomeConfig<C>;

    type GasConfig = GasConfig<C::GasUnit>;

    type CallMessage = CallMessage<C>;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage, C::GasUnit>,
    ) -> Result<(), Error> {
        todo!()
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage, C::GasUnit>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        working_set.deduct_gas(&self.gas_config.method_1_cost)?;

        //  <Self::Context as sov_modules_api::Spec>::Hasher::hash(&[0; 32], working_set);
        self.some_state_value.set(&22, working_set);
        todo!()
    }
}
