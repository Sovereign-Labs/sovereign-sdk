//! Tests that the `codec_builder` feature of the `#[state]` attribute works
//! correctly i.e. the specified builder is used instead of `Default::default`.

use std::marker::PhantomData;

use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::{ModuleId, ModuleInfo, Spec, StateCheckpoint, StateValue};
use sov_state::{DefaultStorageSpec, StateCodec, StateItemDecoder, StateItemEncoder, ZkStorage};
use sov_test_utils::{TestHasher, ZkTestSpec};

#[derive(Clone, ModuleInfo)]
struct TestModule<S: Spec> {
    #[id]
    id: ModuleId,
    #[state(codec_builder = "CustomCodec::new")]
    state_value: StateValue<u32, CustomCodec>,
    #[phantom]
    phantom: PhantomData<S>,
}

#[derive(Default, Clone)]
struct CustomCodec(u32);

impl CustomCodec {
    fn new() -> Self {
        CustomCodec(42)
    }
}

impl StateCodec for CustomCodec {
    type KeyCodec = Self;
    type ValueCodec = Self;

    fn key_codec(&self) -> &Self::KeyCodec {
        self
    }
    fn value_codec(&self) -> &Self::ValueCodec {
        self
    }
}

impl<V> StateItemEncoder<V> for CustomCodec {
    fn encode(&self, _value: &V) -> Vec<u8> {
        std::env::set_var("TEST", self.0.to_string());
        vec![]
    }
}

impl<V> StateItemDecoder<V> for CustomCodec {
    type Error = String;

    fn try_decode(&self, _bytes: &[u8]) -> Result<V, Self::Error> {
        unimplemented!()
    }
}

#[test]
fn custom_builder_works() {
    let storage: ZkStorage<DefaultStorageSpec<TestHasher>> = ZkStorage::new();
    let mut module: TestModule<ZkTestSpec> = TestModule::default();

    let mut state: StateCheckpoint<ZkTestSpec> =
        StateCheckpoint::new(storage, &MockKernel::<ZkTestSpec>::default());
    module.state_value.set(&0u32, &mut state).unwrap();

    assert_eq!(std::env::var("TEST").unwrap(), "42");
}
