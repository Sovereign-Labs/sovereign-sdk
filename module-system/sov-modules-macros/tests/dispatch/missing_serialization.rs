use sov_modules_api::DispatchCall;

#[derive(DispatchCall)]
struct TestStruct {}

#[derive(DispatchCall)]
#[serialization(Serialize, SomethingElse)]
struct TestStruct2 {}

#[derive(DispatchCall)]
#[serialization(OnlySomethingElse)]
struct TestStruct3 {}

#[derive(DispatchCall)]
#[serialization(Serialize, Deserialize, TryToInjectSomethingForbidden)]
struct TestStruct4 {}

fn main() {}
