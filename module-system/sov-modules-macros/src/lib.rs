//! Procedural macros to assist in the creation of Sovereign modules.
//!
//! This crate is not intended to be used directly, please refer to the
//! documentation of [`sov_modules_api`](https://docs.rs/sov-modules-api) for
//! more information with the `macros` feature flag.

// This crate is `missing_docs` because it is not intended to be used directly,
// but only through the re-exports in `sov_modules_api`. All re-exports are
// documented there.
#![allow(missing_docs)]

#[cfg(feature = "native")]
mod cli_parser;
mod common;
mod default_runtime;
mod dispatch;
mod manifest;
mod module_call_json_schema;
mod module_info;
#[cfg(feature = "native")]
mod rpc;

#[cfg(feature = "native")]
use cli_parser::{derive_cli_wallet_arg, CliParserMacro};
use default_runtime::DefaultRuntimeMacro;
use dispatch::dispatch_call::DispatchCallMacro;
use dispatch::genesis::GenesisMacro;
use dispatch::message_codec::MessageCodec;
use module_call_json_schema::derive_module_call_json_schema;
use proc_macro::TokenStream;
#[cfg(feature = "native")]
use rpc::ExposeRpcMacro;
use syn::parse_macro_input;

#[proc_macro_derive(ModuleInfo, attributes(state, module, address))]
pub fn module_info(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);

    handle_macro_error(module_info::derive_module_info(input))
}

#[proc_macro_derive(DefaultRuntime)]
pub fn default_runtime(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let default_config_macro = DefaultRuntimeMacro::new("DefaultRuntime");

    handle_macro_error(default_config_macro.derive_default_runtime(input))
}

#[proc_macro_derive(Genesis)]
pub fn genesis(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let genesis_macro = GenesisMacro::new("Genesis");

    handle_macro_error(genesis_macro.derive_genesis(input))
}

#[proc_macro_derive(DispatchCall, attributes(serialization))]
pub fn dispatch_call(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let call_macro = DispatchCallMacro::new("Call");

    handle_macro_error(call_macro.derive_dispatch_call(input))
}

#[proc_macro_derive(ModuleCallJsonSchema)]
pub fn module_call_json_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    handle_macro_error(derive_module_call_json_schema(input))
}

/// Adds encoding functionality to the underlying type.
#[proc_macro_derive(MessageCodec)]
pub fn codec(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let codec_macro = MessageCodec::new("MessageCodec");

    handle_macro_error(codec_macro.derive_message_codec(input))
}

/// Derives a [`jsonrpsee`] implementation for the underlying type. Any code relying on this macro
/// must take jsonrpsee as a dependency with at least the following features enabled: `["macros", "client-core", "server"]`.
///
/// Syntax is identical to `jsonrpsee`'s `#[rpc]` execept that:
/// 1. `#[rpc]` is renamed to `#[rpc_gen]` to avoid confusion with `jsonrpsee`'s `#[rpc]`
/// 2. `#[rpc_gen]` is applied to an `impl` block instead of a trait
/// 3. `#[method]` is renamed to with `#[rpc_method]` to avoid import confusion and clarify the purpose of the annotation
///
/// ## Example
/// ```
/// use sov_modules_api::{Context, ModuleInfo};
/// use sov_modules_api::macros::rpc_gen;
/// use jsonrpsee::core::RpcResult;
///
/// #[derive(ModuleInfo)]
/// struct MyModule<C: Context> {
///     #[address]
///     addr: C::Address,
///     // ...
/// }
///
/// #[rpc_gen(client, server, namespace = "myNamespace")]
/// impl<C: Context> MyModule<C> {
///     #[rpc_method(name = "myMethod")]
///     fn my_method(&self, param: u32) -> RpcResult<u32> {
///         Ok(1)
///     }
/// }
/// ```
///
/// This is exactly equivalent to hand-writing
///
/// ```
/// use sov_modules_api::{Context, ModuleInfo};
/// use sov_modules_api::macros::rpc_gen;
/// use sov_state::WorkingSet;
/// use jsonrpsee::core::RpcResult;
///
/// #[derive(ModuleInfo)]
/// struct MyModule<C: Context> {
///     #[address]
///     addr: C::Address,
///     // ...
/// };
///
/// impl<C: Context> MyModule<C> {
///     fn my_method(&self, working_set: &mut WorkingSet<C::Storage>, param: u32) -> RpcResult<u32> {
///         Ok(1)
///     }  
/// }
///
/// #[jsonrpsee::proc_macros::rpc(client, server, namespace ="myNamespace")]
/// pub trait MyModuleRpc<C: Context> {
///     #[method(name = "myMethod")]
///     fn my_method(&self, param: u32) ->RpcResult<u32>;
///
///     #[method(name = "health")]
///     fn health(&self) -> RpcResult<()> {
///         Ok(())
///     }
///
///     #[method(name = "moduleAddress")]
///     fn module_address(&self) -> ::jsonrpsee::core::RpcResult<String> {
///        Ok(<MyModule<C> as ModuleInfo>::address(&<MyModule<C> as ::core::default::Default>::default()).to_string())
///     }
///         
/// }
/// ```
///
/// This proc macro also generates an implementation trait intended to be used by a Runtime struct. This trait
/// is named `MyModuleRpcImpl`, and allows a Runtime to be converted into a functional RPC server
/// by simply implementing the two required methods - `get_backing_impl(&self) -> MyModule` and `get_working_set(&self) -> ::sov_modules_api::WorkingSet<C>`
///
/// ```rust,ignore
/// pub trait MyModuleRpcImpl<C: sov_modules_api::Context> {
///     fn get_backing_impl(&self) -> &TestStruct<C>;
///     fn get_working_set(&self) -> ::sov_modules_api::WorkingSet<C>;
///     fn my_method(&self, param: u32) -> u32 {
///         Self::get_backing_impl(self).my_method(self, &mut Self::get_working_set(self), param)
///     }
/// }
/// ```
#[proc_macro_attribute]
#[cfg(feature = "native")]
pub fn rpc_gen(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr: Vec<syn::NestedMeta> = parse_macro_input!(attr);
    let input = parse_macro_input!(item as syn::ItemImpl);
    handle_macro_error(rpc::rpc_gen(attr, input).map(|ok| ok.into()))
}

fn handle_macro_error(result: Result<proc_macro::TokenStream, syn::Error>) -> TokenStream {
    match result {
        Ok(ok) => ok,
        Err(err) => err.to_compile_error().into(),
    }
}

#[cfg(feature = "native")]
#[proc_macro_attribute]
pub fn expose_rpc(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let original = input.clone();
    let input = parse_macro_input!(input);
    let expose_macro = ExposeRpcMacro::new("Expose");
    handle_macro_error(expose_macro.generate_rpc(original, input))
}

#[cfg(feature = "native")]
#[proc_macro_derive(CliWallet, attributes(cli_skip))]
pub fn cli_parser(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let cli_parser = CliParserMacro::new("Cmd");
    handle_macro_error(cli_parser.cli_macro(input))
}
#[cfg(feature = "native")]
#[proc_macro_derive(CliWalletArg)]
pub fn custom_enum_clap(input: TokenStream) -> TokenStream {
    let input: syn::DeriveInput = parse_macro_input!(input);
    handle_macro_error(derive_cli_wallet_arg(input))
}
