## Enabling RPC via Module System Macros

In the Module System, we provide handy macros to make it easy to generate RPC server implementations. In this document,
we'll walk you through all of the steps that you need to take to enable RPC if you're implementing your rollup
from scratch.

There are 5 steps that need to be completed to enable RPC on the full node:

1. Annotate you modules with `rpc_gen` and `rpc_method`.
2. Annotate your `native` `Runtime` with the `expose_rpc` macro.
3. Implement the `RpcRunner` trait on your `StateTransitionRunner`.
4. Import and call `get_rpc_methods` in your full node implementation.
5. Configure and start your RPC server in your full node implementation.

### Step 1: Generate an RPC Server for your Module

To add an RPC method to a module, simply annotate the desired `impl` block with the `rpc_gen` macro and tag each
method you want to expose with the `rpc_method` annotation. As noted in its `rustdoc`s, the `rpc_gen` macro
has identical syntax to [`jsonrpsee::rpc`](https://docs.rs/jsonrpsee-proc-macros/0.18.2/jsonrpsee_proc_macros/attr.rpc.html)
except that the `method` annotation has been renamed to `rpc_method` to clarify its purpose.

```rust
// This code goes in your module's query.rs file
use sov_modules_macros::rpc_gen;

#[rpc_gen(client, server, namespace = "bank")]
impl<C: Context> Bank<C> {
    #[rpc_method(name = "balanceOf")]
    pub(crate) fn balance_of(
        &self,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) ->  BalanceResponse {
    ...
    }

    #[rpc_method(name = "supplyOf")]
    pub(crate) fn supply_of(
        &self,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> TotalSupplyResponse {
     ...
    }
}
```

This example code will generate an RPC module which can process the `bank_balanceOf` and `bank_supplyOf` queries.

Under the hood `rpc_gen` and `rpc_method` create two traits - one called <module_name>RpcImpl and one called <module_name>RpcServer.
It's important to note that the \_RpcImpl and \_RpcServer traits do not need to be implemented - this is done automatically by the SDK.
However, the do need to be imported to the file where the `expose_rpc` macro is called.

### Step 2: Expose Your RPC Server

The next layer of abstraction where we need to think about RPC is the `Runtime`. Just because a module defines
some RPC methods doesn't necessarily mean that we want to use them. So, when we're building a `Runtime`, we have
to enable RPC servers of the modules.

```rust
// This code goes in your state transition function crate. For example demo-stf/runtime.rs

use sov_bank::{BankRpcImpl, BankRpcServer};

#[cfg_attr(
    feature = "native",
    expose_rpc(DefaultContext)
)]
#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Runtime<C: Context> {
    pub bank: sov_bank::Bank<C>,
    ...
}
```

Note that`expose_rpc` takes a tuple as argument, each element of the tuple is a concrete Context.

### Step 3: Implement RpcRunner

Next, we implement the `RpcRunner` trait on our `StateTransitionRunner`. If `expose_rpc` dictates which module RPCs we want to
enable, `RpcRunner` dictates what kind of state they have access to. In this example, we'll expose the current state of our
rollup by giving the `RpcRunner` a handle to our working database. However, we could just as easily use a different storage instance.
For example, we might want to use a read-only database snapshot, which would prevent contention between transaction execution
and RPC queries.

```rust
// This code goes in your state transition function crate. For example demo-stf/app.rs
use sov_modules_api::RpcRunner;
impl<Vm: Zkvm> RpcRunner for DemoAppRunner<DefaultContext, Vm> {
    type Context = DefaultContext;
    fn get_storage(&self) -> <Self::Context as Spec>::Storage {
        self.inner().current_storage.clone()
    }
}
```

### Step 4: Instantiate RPC Methods

Now that we've implemented all of the necessary traits, a `get_rpc_methods` function will be auto-generated.
To use it, simply import it from your state transition function. Given access to `Storage`, this function instantiates
[`jsonrpsee::Methods`](https://docs.rs/jsonrpsee/latest/jsonrpsee/struct.Methods.html) which your full node can
execute. Thanks to the `RpcRunner` trait we just implemented, our full node now has easy access to an appropriate
storage instance.

```rust
// This code goes in your full node implementation. For example demo-rollup/main.rs
use demo_stf::runtime::get_rpc_methods;
use sov_modules_api::RpcRunner;

#[tokio::main]
fn main() {
	// ...
    let mut demo_runner = NativeAppRunner...;

    let storage = demo_runner.get_storage();
    let methods = get_rpc_methods(storage);
	// ...
}
```

### Step 5: Start the Server

The last step is simply binding our generated `jsonrpsee::Methods` to a port:

```rust
async fn start_rpc_server(methods: RpcModule<()>, address: SocketAddr) {
    let server = jsonrpsee::server::ServerBuilder::default()
        .build([address].as_ref())
        .await
        .unwrap();
    let _server_handle = server.start(methods).unwrap();
    futures::future::pending::<()>().await;
}

#[tokio::main]
fn main() {
	// ...
    let mut demo_runner = NativeAppRunner...;

    let storage = demo_runner.get_storage();
    let methods = get_rpc_methods(storage);

    let _handle = tokio::spawn(async move {
        start_rpc_server(methods, address).await;
    });

}
```
