use sov_modules_api::mocks::DefaultContext;
use sov_modules_macros::rpc_gen;
use sov_state::{ProverStorage, WorkingSet};

pub struct TestStruct<C: sov_modules_api::Context> {
    pub phantom: std::marker::PhantomData<C>,
}

#[rpc_gen(client, server)]
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

impl TestStructRpcImpl<DefaultContext> for TestRuntime<DefaultContext> {
    fn get_backing_impl(&self) -> &TestStruct<DefaultContext> {
        &self.test_struct
    }
    fn get_working_set(&self) -> WorkingSet<<DefaultContext as sov_modules_api::Spec>::Storage> {
        let native_storage = ProverStorage::temporary();
        WorkingSet::new(native_storage)
    }
}

fn main() {
    let runtime: TestRuntime<DefaultContext> = TestRuntime {
        test_struct: TestStruct {
            phantom: std::marker::PhantomData,
        },
    };
    {
        let result =
            <TestRuntime<DefaultContext> as TestStructRpcServer<DefaultContext>>::first_method(
                &runtime,
            );
        assert_eq!(result.unwrap(), 11);
    }

    {
        let result =
            <TestRuntime<DefaultContext> as TestStructRpcServer<DefaultContext>>::second_method(
                &runtime, 22,
            );
        assert_eq!(result.unwrap(), 22);
    }

    {
        let result =
            <TestRuntime<DefaultContext> as TestStructRpcServer<DefaultContext>>::third_method(
                &runtime, 33,
            );
        assert_eq!(result.unwrap(), 33);
    }

    {
        let result =
            <TestRuntime<DefaultContext> as TestStructRpcServer<DefaultContext>>::fourth_method(
                &runtime, 44,
            );
        assert_eq!(result.unwrap(), 44);
    }

    {
        let result =
            <TestRuntime<DefaultContext> as TestStructRpcServer<DefaultContext>>::health(&runtime);
        assert_eq!(result.unwrap(), ());
    }

    println!("All tests passed!")
}
