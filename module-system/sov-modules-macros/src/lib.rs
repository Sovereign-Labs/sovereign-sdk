//! Procedural macros to assist in the creation of Sovereign modules.

#![deny(missing_docs)]

#[cfg(feature = "native")]
mod cli_parser;
mod common;
mod default_runtime;
mod dispatch;
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

/// Derives the [`ModuleInfo`](trait.ModuleInfo.html) trait for the underlying `struct`.
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
/// In addition to implementing [`ModuleInfo`](trait.ModuleInfo.html), this macro will
/// also generate so-called "prefix" methods.
///
/// ## Example
///
/// ```
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

/// Derives a custom [`Default`] implementation for the underlying type.
/// We decided to implement a custom macro DefaultRuntime that would implement a custom Default
/// trait for the Runtime because the stdlib implementation of the default trait imposes the generic
/// arguments to have the Default trait, which is not needed in our case.
#[proc_macro_derive(DefaultRuntime)]
pub fn default_runtime(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let default_config_macro = DefaultRuntimeMacro::new("DefaultRuntime");

    handle_macro_error(default_config_macro.derive_default_runtime(input))
}

/// Derives the [`Genesis`](trait.Genesis.html) trait for the underlying runtime
/// `struct`.
#[proc_macro_derive(Genesis)]
pub fn genesis(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let genesis_macro = GenesisMacro::new("Genesis");

    handle_macro_error(genesis_macro.derive_genesis(input))
}

/// Derives the [`DispatchCall`](trait.DispatchCall.html) trait for the underlying
/// type.
#[proc_macro_derive(DispatchCall, attributes(serialization))]
pub fn dispatch_call(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let call_macro = DispatchCallMacro::new("Call");

    handle_macro_error(call_macro.derive_dispatch_call(input))
}

/// Derives the [`ModuleCallJsonSchema`](trait.ModuleCallJsonSchema.html) trait for
/// the underlying type.
///
/// ## Example
///
/// ```
/// use std::marker::PhantomData;
///
/// use sov_modules_api::{Context, Module, ModuleInfo, ModuleCallJsonSchema};
/// use sov_modules_api::default_context::ZkDefaultContext;
/// use sov_state::StateMap;
/// use sov_bank::CallMessage;
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
/// pub trait MyModuleRpc {
///     #[method(name = "myMethod")]
///     fn my_method(&self, param: u32) ->RpcResult<u32>;
///
///     #[method(name = "health")]
///     fn health(&self) -> RpcResult<()> {
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

/// This proc macro generates the actual implementations for the trait created above for the module
/// It iterates over each struct
#[cfg(feature = "native")]
#[proc_macro_attribute]
pub fn expose_rpc(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let original = input.clone();
    let input = parse_macro_input!(input);
    let expose_macro = ExposeRpcMacro::new("Expose");
    handle_macro_error(expose_macro.generate_rpc(original, input))
}

/// Implements the `sov_modules_api::CliWallet` trait for the annotated runtime.
/// Under the hood, this macro generates an enum called `CliTransactionParser` which derives the [`clap::Parser`] trait.
/// This enum has one variant for each field of the `Runtime`, and uses the `sov_modules_api::CliWalletArg` trait to parse the
/// arguments for each of these structs.
///
/// To exclude a module from the CLI, use the `#[cli_skip]` attribute.
///
/// ## Examples
/// ```
/// use sov_modules_api::{Context, DispatchCall, MessageCodec};
/// use sov_modules_api::default_context::DefaultContext;
/// use sov_modules_api::macros::CliWallet;
///
/// #[derive(DispatchCall, MessageCodec, CliWallet)]
/// #[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
/// pub struct Runtime<C: Context> {
///     pub bank: sov_bank::Bank<C>,
///     // ...
/// }
/// ```
#[cfg(feature = "native")]
#[proc_macro_derive(CliWallet, attributes(cli_skip))]
pub fn cli_parser(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let cli_parser = CliParserMacro::new("Cmd");
    handle_macro_error(cli_parser.cli_macro(input))
}

/// Implement [`sov_modules_api::CliWalletArg`] for the annotated struct or enum. Unions are not supported.
///
/// Under the hood, this macro generates a new struct or enum which derives the [`clap::Parser`] trait, and then implements the
/// [`sov_modules_api::CliWalletArg`] trait where the `CliStringRepr` type is the new struct or enum.
///
/// As an implementation detail, `clap` requires that all types have named fields - so this macro auto generates an appropriate
/// `clap`-compatible type from the annotated item. For example, the struct `MyStruct(u64, u64)` would be transformed into
/// `MyStructWithNamedFields { field0: u64, field1: u64 }`.
///
/// ## Example
///
/// This code..
/// ```rust
/// use sov_modules_api::macros::CliWalletArg;
/// #[derive(CliWalletArg, Clone)]
/// pub enum MyEnum {
///    /// A number
///    Number(u32),
///    /// A hash
///    Hash { hash: String },
/// }
/// ```
///
/// ...expands into the following code:
/// ```rust,ignore
/// // The original enum definition is left in its original place
/// pub enum MyEnum {
///    /// A number
///    Number(u32),
///    /// A hash
///    Hash { hash: String },
/// }
///
/// // We generate a new enum with named fields which can derive `clap::Parser`.
/// // Since this variant is only ever converted back to the original, we
/// // don't carry over any of the original derives. However, we do preserve
/// // doc comments from the original version so that `clap` can display them.
/// #[derive(::clap::Parser)]
/// pub enum MyEnumWithNamedFields {
///    /// A number
///    Number { field0: u32 } ,
///    /// A hash
///    Hash { hash: String },
/// }
/// // We generate a `From` impl to convert between the types.
/// impl From<MyEnumWithNamedFields> for MyEnum {
///    fn from(item: MyEnumWithNamedFields) -> Self {
///       match item {
///         Number { field0 } => MyEnum::Number(field0),
///         Hash { hash } => MyEnum::Hash { hash },
///       }
///    }
/// }
///
/// impl sov_modules_api::CliWalletArg for MyEnum {
///     type CliStringRepr = MyEnumWithNamedFields;
/// }
/// ```
#[cfg(feature = "native")]
#[proc_macro_derive(CliWalletArg)]
pub fn custom_enum_clap(input: TokenStream) -> TokenStream {
    let input: syn::DeriveInput = parse_macro_input!(input);
    handle_macro_error(derive_cli_wallet_arg(input))
}
