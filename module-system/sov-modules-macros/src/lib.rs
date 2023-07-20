//! Procedural macros to assist in the creation of Sovereign modules.

#![deny(missing_docs)]
#![feature(log_syntax)]

mod cli_parser;
mod common;
mod default_runtime;
mod dispatch;
mod module_call_json_schema;
mod module_info;
mod rpc;

use cli_parser::{derive_clap_custom_enum, CliParserMacro};
use default_runtime::DefaultRuntimeMacro;
use dispatch::dispatch_call::DispatchCallMacro;
use dispatch::genesis::GenesisMacro;
use dispatch::message_codec::MessageCodec;
use module_call_json_schema::derive_module_call_json_schema;
use proc_macro::TokenStream;
use rpc::ExposeRpcMacro;
use syn::{parse_macro_input, DeriveInput};

/// Derives the [`sov_modules_api::ModuleInfo`] trait for the underlying `struct`.
///
/// The underlying type must respect the following conditions, or compilation
/// will fail:
/// - It must be a named `struct`. Tuple `struct`s, `enum`s, and others are
/// not supported.
/// - It must have *exactly one* field with the `#[address]` attribute. This field
///   represents the **module address**.
/// - All other fields must have either the `#[state]` or `#[module]` attribute.
///   - `#[state]` is used for state members.
///   - `#[module]` is used for module members.
///
/// In addition to implementing [`sov_modules_api::ModuleInfo`], this macro will
/// also generate so-called "prefix" methods. See the [`sov_modules_api`] docs
/// for more information about prefix methods.
///
/// ## Example
///
/// ```
/// use sov_modules_macros::ModuleInfo;
/// use sov_modules_api::{Context, ModuleInfo};
/// use sov_state::StateMap;
///
/// #[derive(ModuleInfo)]
/// struct TestModule<C: Context> {
///     #[address]
///     admin: C::Address,
///
///     #[state]
///     pub state_map: StateMap<String, u32>,
/// }
///
/// // You can then get the prefix of `state_map` like this:
/// fn get_prefix<C: Context>(some_storage: C::Storage) {
///     let test_struct = TestModule::<C>::default();
///     let prefix1 = test_struct.state_map.prefix();
/// }
/// ```
#[proc_macro_derive(ModuleInfo, attributes(state, module, address))]
pub fn module_info(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);

    handle_macro_error(module_info::derive_module_info(input))
}

/// Derives the `sov-modules-api::Default` implementation for the underlying type.
/// We decided to implement a custom macro DefaultRuntime that would implement a custom Default
/// trait for the Runtime because the stdlib implementation of the default trait imposes the generic
/// arguments to have the Default trait, which is not needed in our case.
#[proc_macro_derive(DefaultRuntime)]
pub fn default_runtime(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let default_config_macro = DefaultRuntimeMacro::new("DefaultRuntime");

    handle_macro_error(default_config_macro.derive_default_runtime(input))
}

/// Derives the [`sov_modules_api::Genesis`] trait for the underlying runtime
/// `struct`.
#[proc_macro_derive(Genesis)]
pub fn genesis(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let genesis_macro = GenesisMacro::new("Genesis");

    handle_macro_error(genesis_macro.derive_genesis(input))
}

/// Derives the [`sov_modules_api::DispatchCall`] trait for the underlying type.
#[proc_macro_derive(DispatchCall, attributes(serialization))]
pub fn dispatch_call(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let call_macro = DispatchCallMacro::new("Call");

    handle_macro_error(call_macro.derive_dispatch_call(input))
}

/// Derives the [`sov_modules_api::ModuleCallJsonSchema`] trait for the underlying type.
///
/// ## Example
///
/// ```
/// use std::marker::PhantomData;
///
/// use sov_modules_api::{Context, Module, ModuleInfo, ModuleCallJsonSchema};
/// use sov_modules_api::default_context::ZkDefaultContext;
/// use sov_modules_macros::{ModuleInfo, ModuleCallJsonSchema};
/// use sov_state::StateMap;
/// use sov_bank::call::CallMessage;
///
/// #[derive(ModuleInfo, ModuleCallJsonSchema)]
/// struct TestModule<C: Context> {
///     #[address]
///     admin: C::Address,
///
///     #[state]
///     pub state_map: StateMap<String, u32>,
/// }
///
/// impl<C: Context> Module for TestModule<C> {
///     type Context = C;
///     type Config = PhantomData<C>;
///     type CallMessage = CallMessage<C>;
/// }
///
/// println!("JSON Schema: {}", TestModule::<ZkDefaultContext>::json_schema());
/// ```
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
/// use sov_modules_macros::{rpc_gen, ModuleInfo};
/// use sov_modules_api::Context;
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
///     fn my_method(&self, param: u32) -> u32 {
///         1
///     }
/// }
/// ```
///
/// This is exactly equivalent to hand-writing
///
/// ```
/// use sov_modules_macros::{rpc_gen, ModuleInfo};
/// use sov_modules_api::Context;
/// use sov_state::WorkingSet;
///
/// #[derive(ModuleInfo)]
/// struct MyModule<C: Context> {
///     #[address]
///     addr: C::Address,
///     // ...
/// };
///
/// impl<C: Context> MyModule<C> {
///     fn my_method(&self, working_set: &mut WorkingSet<C::Storage>, param: u32) -> u32 {
///         1
///     }  
/// }
///
/// #[jsonrpsee::proc_macros::rpc(client, server, namespace ="myNamespace")]
/// pub trait MyModuleRpc {
///     #[method(name = "myMethod")]
///     fn my_method(&self, param: u32) -> Result<u32, jsonrpsee::core::Error>;
///
///     #[method(name = "health")]
///     fn health(&self) -> Result<(), jsonrpsee::core::Error> {
///         Ok(())
///     }
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
pub fn rpc_gen(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::ItemImpl);
    handle_macro_error(rpc::rpc_gen(attr.into(), input).map(|ok| ok.into()))
}

fn handle_macro_error(result: Result<proc_macro::TokenStream, syn::Error>) -> TokenStream {
    match result {
        Ok(ok) => ok,
        Err(err) => err.to_compile_error().into(),
    }
}

/// This proc macro generates the actual implementations for the trait created above for the module
/// It iterates over each struct
#[proc_macro_attribute]
pub fn expose_rpc(attr: TokenStream, input: TokenStream) -> TokenStream {
    let context_type = parse_macro_input!(attr);

    let original = input.clone();
    let input = parse_macro_input!(input);
    let expose_macro = ExposeRpcMacro::new("Expose");
    handle_macro_error(expose_macro.generate_rpc(original, input, context_type))
}

/// Generates a CLI arguments parser for the specified runtime.
///
/// ## Examples
/// ```
/// use sov_modules_api::Context;
/// use sov_modules_api::default_context::DefaultContext;
/// use sov_modules_macros::{DispatchCall, MessageCodec, cli_parser};
///
/// #[derive(DispatchCall, MessageCodec)]
/// #[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
/// #[cli_parser]
/// pub struct Runtime<C: Context> {
///     pub bank: sov_bank::Bank<C>,
///     // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn cli_parser(attr: TokenStream, input: TokenStream) -> TokenStream {
    let attrs = parse_macro_input!(attr as syn::AttributeArgs);
    let input = parse_macro_input!(input);

    // let mut context_type: Option<syn::Type> = None;
    let mut skip_fields = Vec::new();

    // Parse attributes
    for attr in attrs {
        if let syn::NestedMeta::Lit(syn::Lit::Str(lit_str)) = attr {
            skip_fields = lit_str.value().split(',').map(|s| s.to_string()).collect();
        }
    }

    // let context_type = context_type.expect("No context type provided");

    let cli_parser = CliParserMacro::new("Cmd");

    handle_macro_error(cli_parser.cli_macro(input, skip_fields))
}

/// Allows the underlying enum to be used as an argument in the sov-cli wallet.
///
/// Under the hood, this macro generates an enum with unnamed fields
#[proc_macro_derive(CliWalletArg, attributes(module_name))]
pub fn custom_enum_clap(input: TokenStream) -> TokenStream {
    let input: DeriveInput = parse_macro_input!(input);
    match input.data {
        syn::Data::Struct(_) => todo!(),
        syn::Data::Enum(_) => handle_macro_error(derive_clap_custom_enum(input)),
        syn::Data::Union(_) => todo!(),
        // ;
    }
}

/// Causes the annotated module to be excluded from the generated CLI.
/// This annotation is typically used for modules which don't directly accept
/// on-chain transactions.
#[proc_macro_attribute]
pub fn cli_skip(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
