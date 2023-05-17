use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::RpcStorage;
use sov_modules_macros::ModuleInfo;
use sov_modules_macros::rpc_gen;
use sov_state::{ProverStorage, WorkingSet};

#[derive(ModuleInfo)]
pub struct TestStruct<C: ::sov_modules_api::Context> {
    #[address]
    pub(crate) address: C::Address,
}

#[rpc_gen(client, server, namespace = "test")]
impl<C: sov_modules_api::Context> TestStruct<C> {
    #[rpc_method(name = "firstMethod")]
    pub fn first_method(&self, _working_set: &mut WorkingSet<C::Storage>) -> u32 {
        11
    }

    #[rpc_method(name = "secondMethod")]
    pub fn second_method(&self, result: u32, _working_set: &mut WorkingSet<C::Storage>) -> u32 {
        result
    }

    #[rpc_method(name = "thirdMethod")]
    pub fn third_method(&self, result: u32) -> u32 {
        result
    }

    #[rpc_method(name = "fourthMethod")]
    pub fn fourth_method(&self, _working_set: &mut WorkingSet<C::Storage>, result: u32) -> u32 {
        result
    }
}

pub struct TestRuntime<C: sov_modules_api::Context> {
    test_struct: TestStruct<C>,
}

impl TestStructRpcImpl<DefaultContext>
for ::sov_modules_api::RpcStorage<DefaultContext> {
    fn get_working_set(
        &self,
    ) -> ::sov_state::WorkingSet<<DefaultContext as ::sov_modules_api::Spec>::Storage> {
        ::sov_state::WorkingSet::new(self.storage.clone())
    }
}


fn main() {
    let native_storage = ProverStorage::temporary();
    let r: RpcStorage<DefaultContext> = RpcStorage { storage: native_storage.clone() };
    {
        let result =
            <RpcStorage<DefaultContext> as TestStructRpcServer<DefaultContext>>::first_method(
                &r,
            );
        assert_eq!(result.unwrap(), 11);
    }

    {
        let result =
            <RpcStorage<DefaultContext> as TestStructRpcServer<DefaultContext>>::second_method(
                &r, 22,
            );
        assert_eq!(result.unwrap(), 22);
    }

    {
        let result =
            <RpcStorage<DefaultContext> as TestStructRpcServer<DefaultContext>>::third_method(
                &r, 33,
            );
        assert_eq!(result.unwrap(), 33);
    }

    {
        let result =
            <RpcStorage<DefaultContext> as TestStructRpcServer<DefaultContext>>::fourth_method(
                &r, 44,
            );
        assert_eq!(result.unwrap(), 44);
    }

    {
        let result =
            <RpcStorage<DefaultContext> as TestStructRpcServer<DefaultContext>>::health(&r);
        assert_eq!(result.unwrap(), ());
    }

    println!("All tests passed!")
}
