## Enabling RPC via SDK Macros
There are 5 steps that need to be completed to enable RPC on the full node
1. Annotate the modules that need to expose their data with `rpc_gen` and `rpc_method`
2. Annotate the state transition runner with the specific modules to expose with `expose_rpc`
3. Implement the `RpcRunner` trait. provide an implementation for the `get_storage` function
4. Import and call `get_rpc_module` to get a combined rpc module for the modules annotated and exposed in 1 and 2
5. Use the modules returned from the above function and bind them to an RPC server

### Modules
* We need to annotate the `impl` block for our module. In this case its `ModuleStruct`
```
impl<C: Context> ModuleStruct<C> {
    pub fn first_method(&self, working_set: &mut WorkingSet<C::Storage>) -> u32 {
        42
    }
    pub fn second_method(&self) -> u32 {
        24
    }
}
```
* annotate with `rpc_gen` and `rpc_method`
```
use sov_modules_macros::rpc_gen;

#[rpc_gen(client, server, namespace = "mymodule")]
impl<C: Context> MyModule<C> {
    #[rpc_method(name = "firstMethod")]
    pub fn first_method(&self, working_set: &mut WorkingSet<C::Storage>) -> u32 {
        42
    }
    #[rpc_method(name = "secondMethod")]
    pub fn second_method(&self) -> u32 {
        24
    }
}
```
* `rpc_gen` and `rpc_method` create <module_name>RpcImpl and <module_name>RpcServer traits. 
* The RpcImpl and RpcServer traits do not need to be implemented - this is done automatically by the SDK, but they need to be imported to the file where the `expose_rpc` macro is called
* Once all the modules that need be part of the RPC are annotated, we annotate our Runner struct that impls `StateTransitionRunner` with an `expose_rpc` attribute macro.
```
use bank::query::{MyModuleRpcImpl, MyModuleRpcServer};

#[expose_rpc((MyModuleStruct<DefaultContext>,))]
impl<Vm: Zkvm> StateTransitionRunner<ProverConfig, Vm> for DemoAppRunner<DefaultContext, Vm> {
...
}
```
* `expose_rpc` takes a tuple as arguments. each element of the tuple is a module with a concrete Context.
* next, we implement the `RpcRunner` trait. we do this in the `demo_stf/app.rs` file
```
use sov_modules_api::RpcRunner;
impl<Vm: Zkvm> RpcRunner for DemoAppRunner<DefaultContext, Vm> {
    type Context = DefaultContext;
    fn get_storage(&self) -> <Self::Context as Spec>::Storage {
        self.inner().current_storage.clone()
    }
}
```
* `RpcRunner` primarily need to provide a storage which is used by the RPC server. It's a helper trait
* To start the jsonrpsee server, we need the rpc modules, which are provided by the macro generated method `get_rpc_module`
```
use demo_stf::app::get_rpc_module;

    let mut demo_runner = NativeAppRunner...;

    let storj = demo_runner.get_storage();
    let module = get_rpc_module(storj);
```
* This is the register + network interface binding step, and starting the actual RPC server
```
async fn start_rpc_server(module: RpcModule<()>, address: SocketAddr) {
    let server = jsonrpsee::server::ServerBuilder::default()
        .build([address].as_ref())
        .await
        .unwrap();
    let _server_handle = server.start(module).unwrap();
    futures::future::pending::<()>().await;
}  

    let _handle = tokio::spawn(async move {
        start_rpc_server(module, address).await;
    }); 

```
* we're using `futures::future::pending::<()>().await` to block the spawned RPC server, but this can be implemented in multiple ways
* Another note is that we're configuring address in the `rollup_config.toml`
```
[rpc_config]
# the host and port to bind the rpc server for
bind_host = "127.0.0.1"
bind_port = 12345
```
* The above can be parsed using
```
    let rollup_config: RollupConfig = from_toml_path("rollup_config.toml")?;
    let rpc_config = rollup_config.rpc_config;
    let address = SocketAddr::new(rpc_config.bind_host.parse()?, rpc_config.bind_port);
```
* But as mentioned, the infra / networking aspect is separated from the macro that generates the boilerplate to expose the RPC in a way that it can be plugged into an RPC server