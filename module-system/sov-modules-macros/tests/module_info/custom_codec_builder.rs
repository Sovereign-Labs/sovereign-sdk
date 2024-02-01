use sov_modules_api::{Context, ModuleInfo, StateMap};

#[derive(ModuleInfo)]
struct FirstTestStruct<C>
where
    C: Context,
{
    #[address]
    pub address: C::Address,

    #[state(codec_builder = "sov_state::codec::BorshCodec::default")]
    pub state_in_first_struct_1: StateMap<C::PublicKey, u32>,
}

fn main() {}
