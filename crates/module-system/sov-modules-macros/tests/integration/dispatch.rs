use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::{
    decode_borsh_serialized_message, Context, DaSpec, DispatchCall, EncodeCall, Error, Event,
    Genesis, MessageCodec, Module, ModuleInfo, Spec, StateValue, TxState, WorkingSet,
};
use sov_state::ZkStorage;
use sov_test_utils::{TestSpec, ZkTestSpec};
use third_test_module::ModuleThreeStorable;

pub mod first_test_module {
    use sov_modules_api::ModuleId;

    use super::*;

    #[derive(Clone, ModuleInfo)]
    pub struct FirstTestStruct<S: Spec> {
        #[id]
        pub id: ModuleId,

        #[state]
        pub state_in_first_struct: StateValue<u8>,

        #[phantom]
        phantom: std::marker::PhantomData<S>,
    }

    impl<S: Spec> FirstTestStruct<S> {
        pub fn get_state_value(&self, state: &mut WorkingSet<S>) -> Result<u8, Error> {
            Ok(self
                .state_in_first_struct
                .get(state)
                .map_err(|e| Error::ModuleError(e.into()))?
                .unwrap())
        }
    }

    #[derive(
        borsh::BorshDeserialize,
        borsh::BorshSerialize,
        serde::Serialize,
        serde::Deserialize,
        Debug,
        PartialEq,
        Clone,
        schemars::JsonSchema,
    )]
    pub enum Event {
        FirstModuleEnum1(u64),
        FirstModuleEnum2,
        FirstModuleEnum3(Vec<u8>),
    }

    impl<S: Spec> Module for FirstTestStruct<S> {
        type Spec = S;
        type Config = ();
        type CallMessage = u8;
        type Event = Event;

        fn genesis(
            &mut self,
            _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,

            _config: &Self::Config,
            state: &mut impl sov_modules_api::GenesisState<S>,
        ) -> anyhow::Result<()> {
            self.state_in_first_struct.set(&1, state).unwrap();
            Ok(())
        }

        fn call(
            &mut self,
            msg: Self::CallMessage,
            _context: &Context<Self::Spec>,
            state: &mut impl TxState<S>,
        ) -> anyhow::Result<()> {
            self.state_in_first_struct.set(&msg, state)?;
            Ok(())
        }
    }
}

pub mod second_test_module {
    use sov_modules_api::ModuleId;

    use super::*;

    #[derive(Clone, ModuleInfo)]
    pub struct SecondTestStruct<S: Spec> {
        #[id]
        pub id: ModuleId,

        #[state]
        pub state_in_second_struct: StateValue<u8>,

        #[phantom]
        phantom: std::marker::PhantomData<S>,
    }

    impl<S: Spec> SecondTestStruct<S> {
        pub fn get_state_value(&self, state: &mut WorkingSet<S>) -> Result<u8, Error> {
            Ok(self
                .state_in_second_struct
                .get(state)
                .map_err(|e| Error::ModuleError(e.into()))?
                .unwrap())
        }
    }

    #[derive(
        borsh::BorshDeserialize,
        borsh::BorshSerialize,
        serde::Serialize,
        serde::Deserialize,
        Debug,
        PartialEq,
        Clone,
        schemars::JsonSchema,
    )]
    pub enum Event {
        SecondModuleEnum,
    }

    impl<S: Spec> Module for SecondTestStruct<S> {
        type Spec = S;
        type Config = ();
        type CallMessage = u8;
        type Event = Event;

        fn genesis(
            &mut self,
            _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,

            _config: &Self::Config,
            state: &mut impl sov_modules_api::GenesisState<S>,
        ) -> anyhow::Result<()> {
            self.state_in_second_struct.set(&2, state).unwrap();
            Ok(())
        }

        fn call(
            &mut self,
            msg: Self::CallMessage,
            _context: &Context<Self::Spec>,
            state: &mut impl TxState<S>,
        ) -> anyhow::Result<()> {
            self.state_in_second_struct.set(&msg, state)?;
            Ok(())
        }
    }
}

pub mod third_test_module {
    use sov_modules_api::sov_universal_wallet::schema::UniversalWallet;
    use sov_modules_api::ModuleId;

    use super::*;

    pub trait ModuleThreeStorable:
        borsh::BorshSerialize
        + borsh::BorshDeserialize
        + UniversalWallet
        + schemars::JsonSchema
        + core::fmt::Debug
        + Default
        + PartialEq
        + Eq
        + Clone
        + Send
        + Sync
        + UniversalWallet
        + 'static
    {
    }

    impl ModuleThreeStorable for u32 {}

    #[derive(Clone, ModuleInfo)]
    pub struct ThirdTestStruct<S: Spec, OtherGeneric: ModuleThreeStorable> {
        #[id]
        pub id: ModuleId,

        #[state]
        pub state_in_third_struct: StateValue<OtherGeneric>,

        #[phantom]
        phantom: std::marker::PhantomData<S>,
    }

    impl<S: Spec, OtherGeneric: ModuleThreeStorable> ThirdTestStruct<S, OtherGeneric> {
        pub fn get_state_value(
            &self,
            state: &mut WorkingSet<S>,
        ) -> Result<Option<OtherGeneric>, Error> {
            self.state_in_third_struct
                .get(state)
                .map_err(|e| Error::ModuleError(e.into()))
        }
    }

    #[derive(
        borsh::BorshDeserialize,
        borsh::BorshSerialize,
        serde::Serialize,
        serde::Deserialize,
        Debug,
        PartialEq,
        Clone,
        schemars::JsonSchema,
    )]
    pub enum Event {
        ThirdModuleEnum,
    }

    impl<S: Spec, OtherGeneric: ModuleThreeStorable> Module for ThirdTestStruct<S, OtherGeneric> {
        type Spec = S;
        type Config = ();
        type CallMessage = OtherGeneric;
        type Event = Event;

        fn genesis(
            &mut self,
            _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,

            _config: &Self::Config,
            state: &mut impl sov_modules_api::GenesisState<S>,
        ) -> anyhow::Result<()> {
            self.state_in_third_struct
                .set(&Default::default(), state)
                .unwrap();
            Ok(())
        }

        fn call(
            &mut self,
            msg: Self::CallMessage,
            _context: &Context<Self::Spec>,
            state: &mut impl TxState<S>,
        ) -> anyhow::Result<()> {
            self.state_in_third_struct.set(&msg, state)?;
            Ok(())
        }
    }
}

// Wrap the test in a module rather than declaring the struct inside of the function
// to avoid proc-macro resolution fallback error: https://github.com/rust-lang/rust/issues/83583
mod custom_attributes {
    use super::*;
    #[derive(Default, Genesis, DispatchCall, Event, MessageCodec)]
    struct Runtime<S: Spec> {
        pub first: first_test_module::FirstTestStruct<S>,
        pub second: second_test_module::SecondTestStruct<S>,
    }
    #[test]
    fn custom_attributes() {
        let _ = Schema::of_single_type::<RuntimeCall<TestSpec>>();
    }
}

// Wrap the test in a module rather than declaring the struct inside of the function
// to avoid proc-macro resolution fallback error: https://github.com/rust-lang/rust/issues/83583
mod derive_event {
    use sov_modules_api::NestedEnumUtils;

    use super::*;
    #[derive(Default, Genesis, DispatchCall, Event, MessageCodec)]
    struct Runtime<S: Spec> {
        pub first: first_test_module::FirstTestStruct<S>,
        pub second: second_test_module::SecondTestStruct<S>,
    }

    #[test]
    fn derive_event() {
        // Check to see if the runtime events are getting initialized correctly
        let _event =
            RuntimeEvent::<TestSpec>::First(first_test_module::Event::FirstModuleEnum1(10));
        let _event = RuntimeEvent::<TestSpec>::First(first_test_module::Event::FirstModuleEnum2);
        let _event =
            RuntimeEvent::<TestSpec>::First(first_test_module::Event::FirstModuleEnum3(vec![1; 3]));
        let event = RuntimeEvent::<TestSpec>::Second(second_test_module::Event::SecondModuleEnum);
        let discriminant: &'static str = event.discriminant().into();
        assert_eq!(discriminant, "Second");
    }
}

// Wrap the test in a module rather than declaring the struct inside of the function
// to avoid proc-macro resolution fallback error: https://github.com/rust-lang/rust/issues/83583
mod derive_genesis {
    use super::*;
    #[derive(Default, Genesis, DispatchCall, MessageCodec)]
    struct Runtime<S, T>
    where
        S: Spec,
        T: ModuleThreeStorable,
    {
        pub first: first_test_module::FirstTestStruct<S>,
        pub second: second_test_module::SecondTestStruct<S>,
        pub third: third_test_module::ThirdTestStruct<S, T>,
    }

    #[test]
    fn derive_genesis() {
        let storage = ZkStorage::new();
        let mut state =
            sov_modules_api::StateCheckpoint::new(storage, &MockKernel::<ZkTestSpec>::default());
        let runtime = &mut Runtime::<ZkTestSpec, u32>::default();
        let config = GenesisConfig::new((), (), ());
        let mut genesis_state =
            state.to_genesis_state_accessor::<Runtime<ZkTestSpec, u32>>(&config);
        runtime
            .genesis(&Default::default(), &config, &mut genesis_state)
            .unwrap();
        let mut working_set = state.to_working_set_unmetered();

        {
            let response = runtime
                .first
                .get_state_value(&mut working_set)
                .expect("The working set should be unmetered");
            assert_eq!(response, 1);
        }

        {
            let response = runtime
                .second
                .get_state_value(&mut working_set)
                .expect("The working set should be unmetered");
            assert_eq!(response, 2);
        }

        {
            let response = runtime
                .third
                .get_state_value(&mut working_set)
                .expect("The working set should be unmetered");
            assert_eq!(response, Some(0));
        }
    }
}

// Wrap the test in a module rather than declaring the struct inside of the function
// to avoid proc-macro resolution fallback error: https://github.com/rust-lang/rust/issues/83583
mod derive_dispatch {
    use sov_modules_api::NestedEnumUtils;

    use super::*;
    #[derive(Default, Genesis, DispatchCall, MessageCodec)]
    struct Runtime<S, T>
    where
        S: Spec,
        T: ModuleThreeStorable,
    {
        pub first: first_test_module::FirstTestStruct<S>,
        pub second: second_test_module::SecondTestStruct<S>,
        pub third: third_test_module::ThirdTestStruct<S, T>,
    }

    #[test]
    fn derive_dispatch() {
        type RT = Runtime<ZkTestSpec, u32>;
        type Call = RuntimeCall<ZkTestSpec, u32>;

        let runtime = &mut RT::default();

        let storage = ZkStorage::new();

        let mut state =
            sov_modules_api::StateCheckpoint::new(storage, &MockKernel::<ZkTestSpec>::default());
        let config = GenesisConfig::new((), (), ());
        let mut genesis_state =
            state.to_genesis_state_accessor::<Runtime<ZkTestSpec, u32>>(&config);
        runtime
            .genesis(&Default::default(), &config, &mut genesis_state)
            .unwrap();
        let mut working_set = state.to_working_set_unmetered();

        let sender = <ZkTestSpec as Spec>::Address::from([0; 28]);
        let sequencer = <ZkTestSpec as Spec>::Address::from([1; 28]);
        let sequencer_da = <<ZkTestSpec as Spec>::Da as DaSpec>::Address::new([0; 32]);
        let context: Context<ZkTestSpec> =
            Context::new(sender, Default::default(), sequencer, sequencer_da);

        let value = 11;
        {
            let message = value;
            let serialized_message = <RT as EncodeCall<
                first_test_module::FirstTestStruct<ZkTestSpec>,
            >>::encode_call(message);
            let module = decode_borsh_serialized_message::<<RT as DispatchCall>::Decodable>(
                &serialized_message,
            )
            .unwrap();

            assert_eq!(runtime.module_id(&module), runtime.first.id());
            runtime
                .dispatch_call(module, &mut working_set, &context)
                .unwrap();
        }

        {
            let response = runtime
                .first
                .get_state_value(&mut working_set)
                .expect("The working set should be unmetered");
            assert_eq!(response, value);
        }

        let value = 22;
        {
            let message = value;
            let serialized_message = <RT as EncodeCall<
                second_test_module::SecondTestStruct<ZkTestSpec>,
            >>::encode_call(message);
            let module = decode_borsh_serialized_message::<<RT as DispatchCall>::Decodable>(
                &serialized_message,
            )
            .unwrap();

            assert_eq!(runtime.module_id(&module), runtime.second.id());

            runtime
                .dispatch_call(module, &mut working_set, &context)
                .unwrap();
        }

        {
            let response = runtime
                .second
                .get_state_value(&mut working_set)
                .expect("The working set should be unmetered");
            assert_eq!(response, value);
        }

        {
            let call = Call::Second(12);
            // Check that we can retrieve the module info from the discriminant of a callmessage.
            assert_eq!(
                runtime.module_info(call.discriminant()).id(),
                runtime.second.id()
            );
            let discriminant: &'static str = call.discriminant().into();
            assert_eq!(discriminant, "Second");
        }
    }
}
