use std::panic::catch_unwind;

use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_api::{Context, ModuleInfo};
use sov_state::{DefaultStorageSpec, ProverStorage, StateValue, StateValueCodec, WorkingSet};

#[derive(ModuleInfo)]
struct TestModule<C>
where
    C: Context,
{
    #[address]
    pub address: C::Address,

    #[state(codec_builder = "Default::default")]
    pub state_value: StateValue<u32, CustomCodec>,
}

#[derive(Default)]
pub struct CustomCodec;

impl<V> StateValueCodec<V> for CustomCodec {
    type ValueError = String;

    fn encode_value(&self, _value: &V) -> Vec<u8> {
        unimplemented!()
    }

    fn try_decode_value(&self, _bytes: &[u8]) -> Result<V, Self::ValueError> {
        unimplemented!()
    }
}

#[test]
fn main() {
    let module: TestModule<ZkDefaultContext> = Default::default();

    let tempdir = tempfile::tempdir().unwrap();
    let storage: ProverStorage<DefaultStorageSpec> = ProverStorage::with_path(&tempdir).unwrap();

    catch_unwind(|| {
        let mut working_set = WorkingSet::new(storage);
        module.state_value.set(&0u32, &mut working_set);
    })
    .unwrap_err();
}
