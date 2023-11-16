use sov_modules_api::{
    prelude::*, CallResponse, Context, Error, Module, ModuleInfo, StateValue, WorkingSet,
};

pub mod first_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub struct FirstTestStruct<C: Context> {
        #[address]
        pub address: C::Address,

        #[state]
        pub state_in_first_struct: StateValue<u8>,
    }

    impl<C: Context> FirstTestStruct<C> {
        pub fn get_state_value(&self, working_set: &mut WorkingSet<C>) -> u8 {
            self.state_in_first_struct.get(working_set).unwrap()
        }
    }

    #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
    pub enum Event {
        FirstModuleEnum1(u64),
        FirstModuleEnum2,
        FirstModuleEnum3(Vec<u8>),
    }

    impl<C: Context> Module for FirstTestStruct<C> {
        type Context = C;
        type Config = ();
        type CallMessage = u8;
        type Event = Event;

        fn genesis(
            &self,
            _config: &Self::Config,
            working_set: &mut WorkingSet<C>,
        ) -> Result<(), Error> {
            self.state_in_first_struct.set(&1, working_set);
            Ok(())
        }

        fn call(
            &self,
            msg: Self::CallMessage,
            _context: &Self::Context,
            working_set: &mut WorkingSet<C>,
        ) -> Result<CallResponse, Error> {
            self.state_in_first_struct.set(&msg, working_set);
            Ok(CallResponse::default())
        }
    }
}

pub mod second_test_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub struct SecondTestStruct<C: Context> {
        #[address]
        pub address: C::Address,

        #[state]
        pub state_in_second_struct: StateValue<u8>,
    }

    impl<C: Context> SecondTestStruct<C> {
        pub fn get_state_value(&self, working_set: &mut WorkingSet<C>) -> u8 {
            self.state_in_second_struct.get(working_set).unwrap()
        }
    }

    #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
    pub enum Event {
        SecondModuleEnum,
    }

    impl<Ctx: Context> Module for SecondTestStruct<Ctx> {
        type Context = Ctx;
        type Config = ();
        type CallMessage = u8;
        type Event = Event;

        fn genesis(
            &self,
            _config: &Self::Config,
            working_set: &mut WorkingSet<Ctx>,
        ) -> Result<(), Error> {
            self.state_in_second_struct.set(&2, working_set);
            Ok(())
        }

        fn call(
            &self,
            msg: Self::CallMessage,
            _context: &Self::Context,
            working_set: &mut WorkingSet<Ctx>,
        ) -> Result<CallResponse, Error> {
            self.state_in_second_struct.set(&msg, working_set);
            Ok(CallResponse::default())
        }
    }
}

pub mod third_test_module {
    use super::*;

    pub trait ModuleThreeStorable:
        borsh::BorshSerialize + borsh::BorshDeserialize + core::fmt::Debug + Default + Send + Sync
    {
    }

    impl ModuleThreeStorable for u32 {}

    #[derive(ModuleInfo)]
    pub struct ThirdTestStruct<Ctx: Context, OtherGeneric: ModuleThreeStorable> {
        #[address]
        pub address: Ctx::Address,

        #[state]
        pub state_in_third_struct: StateValue<OtherGeneric>,
    }

    impl<Ctx: Context, OtherGeneric: ModuleThreeStorable> ThirdTestStruct<Ctx, OtherGeneric> {
        pub fn get_state_value(&self, working_set: &mut WorkingSet<Ctx>) -> Option<OtherGeneric> {
            self.state_in_third_struct.get(working_set)
        }
    }

    impl<Ctx: Context, OtherGeneric: ModuleThreeStorable> Module
        for ThirdTestStruct<Ctx, OtherGeneric>
    {
        type Context = Ctx;
        type Config = ();
        type CallMessage = OtherGeneric;
        type Event = ();

        fn genesis(
            &self,
            _config: &Self::Config,
            working_set: &mut WorkingSet<Ctx>,
        ) -> Result<(), Error> {
            self.state_in_third_struct
                .set(&Default::default(), working_set);
            Ok(())
        }

        fn call(
            &self,
            msg: Self::CallMessage,
            _context: &Self::Context,
            working_set: &mut WorkingSet<Ctx>,
        ) -> Result<CallResponse, Error> {
            self.state_in_third_struct.set(&msg, working_set);
            Ok(CallResponse::default())
        }
    }
}
