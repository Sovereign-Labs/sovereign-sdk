#![feature(log_syntax)]
mod dispatch;
mod module_info;
use dispatch::{
    dispatch_call::DispatchCallMacro, dispatch_query::DispatchQueryMacro, genesis::GenesisMacro,
    message_codec::MessageCodec,
};
use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Derives the `sov-modules-api::ModuleInfo` implementation for the underlying type.
///
/// See `sov-modules-api` for definition of `prefix`.
/// ## Example
///
/// ``` ignore
///  #[derive(ModuleInfo)]
///  pub(crate) struct TestModule<C: Context> {
///     #[state]
///     pub test_state1: TestState<C::Storage>,
///
///     #[state]
///     pub test_state2: TestState<C::Storage>,
///  }
/// ```
/// allows getting a prefix of a member field like:
/// ```ignore
///  let test_struct = <TestModule::<SomeContext> as sov_modules_api::ModuleInfo>::new(some_storage);
///  let prefix1 = test_struct.test_state1.prefix;
/// ````
/// ## Attributes
///
///  * `state` - attribute for state members
///  * `module` - attribute for module members
///  * `address` - attribute for module address
#[proc_macro_derive(ModuleInfo, attributes(state, module, address))]
pub fn module_info(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);

    handle_macro_error(module_info::derive_module_info(input))
}

/// Derives the `sov-modules-api::Genesis` implementation for the underlying type.
#[proc_macro_derive(Genesis)]
pub fn genesis(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let genesis_macro = GenesisMacro::new("Genesis");

    handle_macro_error(genesis_macro.derive_genesis(input))
}

/// Derives the `sov-modules-api::DispatchCall` implementation for the underlying type.
#[proc_macro_derive(DispatchCall, attributes(serialization))]
pub fn dispatch_call(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let call_macro = DispatchCallMacro::new("Call");

    handle_macro_error(call_macro.derive_dispatch_call(input))
}

/// Derives the `sov-modules-api::DispatchQuery` implementation for the underlying type.
#[proc_macro_derive(DispatchQuery, attributes(serialization))]
pub fn dispatch_query(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let query_macro = DispatchQueryMacro::new("Query");

    handle_macro_error(query_macro.derive_dispatch_query(input))
}

/// Adds encoding functionality to the underlying type.
#[proc_macro_derive(MessageCodec)]
pub fn codec(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let codec_macro = MessageCodec::new("MessageCodec");

    handle_macro_error(codec_macro.derive_message_codec(input))
}

/// Derive a `jsonrpsee` implementation for the underlying type. Any code relying on this macro
/// must take jsonrpsee as a dependency with at least the following features enabled: `["macros", "client-core", "server"]`.
///
/// Syntax is identical to `jsonrpsee`'s `#[rpc]` execept that:
/// 1. `#[rpc]` is renamed to `#[rpc_gen]` to avoid confusion with `jsonrpsee`'s `#[rpc]`
/// 2. `#[rpc_gen]` is applied to an `impl` block instead of a trait
/// 3. `#[method]` is renamed to with `#[rpc_method]` to avoid import confusion and clarify the purpose of the annotation
///
/// ## Example
///  ```rust,ignore
///  struct MyModule {};
///
/// #[rpc_gen(client, server, namespace ="myNamespace")]
/// impl MyModule {
///    #[rpc_method(name = "myMethod")]
///     fn my_method(&self, param: u32) -> u32 {
///          1
///     }
/// }
/// ```
///
/// This is exactly equivalent to hand-writing
/// ```rust,ignore
/// struct MyModule {};
///
/// impl MyModule {
///     fn my_method(&self, param: u32) -> u32 {
///         1
///     }  
/// }
///
/// #[jsonrpsee::rpc(client, server, namespace ="myNamespace")]
/// pub trait MyModuleRpc {
///    #[jsonrpsee::method(name = "myMethod")]
///    fn my_method(&self, param: u32) -> Result<u32, jsonrpsee::Error>;
///    #[method(name = "health")]
///    fn health() -> Result<(), jsonrpsee::Error> {
///        Ok(())
///    }
/// }
/// ```
#[proc_macro_attribute]
pub fn rpc_gen(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::ItemImpl);
    // dbg!( attr);
    // let attr = parse_macro_input!(attr as syn::AttributeArgs);

    handle_macro_error(dispatch::derive_rpc::derive_rpc(attr.into(), input).map(|ok| ok.into()))
}

fn handle_macro_error(result: Result<proc_macro::TokenStream, syn::Error>) -> TokenStream {
    match result {
        Ok(ok) => ok,
        Err(err) => err.to_compile_error().into(),
    }
}
