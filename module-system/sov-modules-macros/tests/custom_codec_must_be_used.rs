use std::panic::catch_unwind;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Context, ModuleInfo, StateValue, WorkingSet};
use sov_state::codec::{StateCodec, StateKeyCodec, StateValueCodec};
use sov_state::{DefaultStorageSpec, ProverStorage};

#[derive(ModuleInfo)]
struct TestModule<C>
where
    C: Context,
{
    #[address]
    address: C::Address,

    #[state(codec_builder = "crate::CustomCodec::new")]
    state_value: StateValue<u32, CustomCodec>,
}

#[derive(Default)]
struct CustomCodec;

impl CustomCodec {
    fn new() -> Self {
        Self
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

impl<K> StateKeyCodec<K> for CustomCodec {
    fn encode_key(&self, _key: &K) -> Vec<u8> {
        unimplemented!()
    }
}

impl<V> StateValueCodec<V> for CustomCodec {
    type Error = String;

    fn encode_value(&self, _value: &V) -> Vec<u8> {
        unimplemented!()
    }

    fn try_decode_value(&self, _bytes: &[u8]) -> Result<V, Self::Error> {
        unimplemented!()
    }
}

fn main() {
    let tempdir = tempfile::tempdir().unwrap();
    let storage: ProverStorage<DefaultStorageSpec> = ProverStorage::with_path(&tempdir).unwrap();
    let module: TestModule<DefaultContext> = TestModule::default();

    catch_unwind(|| {
        let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(storage);
        module.state_value.set(&0u32, &mut working_set);
    })
    .unwrap_err();
}
