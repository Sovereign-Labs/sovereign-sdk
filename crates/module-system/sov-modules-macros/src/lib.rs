//! Procedural macros to assist in the creation of Sovereign SDK modules and rollups.
//!
//! # Are you lost?
//!
//! This crate is **not** intended to be used directly!
//!
//! **Please refer to the documentation of `sov-modules-api` for
//! more information.**
//!
//! # Notes about authoring proc-macros
//!
//! This section serves as a collection of resources and useful tips for
//! authoring new proc-macros and maintaining the proc-macros present in this
//! crate.
//!
//! <div class="warning">
//! This place is a message...and part of a
//! system of messages...pay attention to it!
//!
//! Sending this message was important to us.
//! We considered ourselves to be a powerful culture.
//!
//! This place is not a place of honor... no
//! highly esteemed deed is commemorated here.
//!
//! What is here was dangerous and repulsive to us.
//! This message is a warning about danger.[^nuclear-warning]
//! </div>
//!
//! [^nuclear-warning]: <https://en.wikipedia.org/wiki/Long-term_nuclear_waste_warning_messages#Message>
//!
//! ## The `#[automatically_derived]` attribute
//!
//! Whenever your proc-macro generates a trait implementation, you should mark
//! it as `#[automatically_derived]`. It's good practice, to help the compiler
//! silence some warnings originating from generated code.
//!
//! See <https://stackoverflow.com/questions/51481551/what-does-automatically-derived-mean>.
//!
//! ## Separate parsing and generation
//!
//! It's almost always a good idea to cleanly separate your proc-macro parsing
//! logic from the code generation logic. Your proc-macro should ideally be composed of:
//!
//! 1. A parsing function, which consumes tokens and returns an instante of some
//!    custom type which provides well-typed informations about the input.
//!    [`darling`] is excellent for this.
//! 2. A code generator, which consumes the well-typed, parsed input and
//!    produces tokens.
//!
//! Besides readability, an extra benefit of this approach is that different
//! proc-macros can invoke each other's parsing logic if needed, and compose
//! nicely.
//!
//! ## Inner vs outer feature gating
//!
//! Sometimes your proc-macro is only intended to be used when a specific Cargo
//! feature is enabled, e.g. `native`. You usually have two choices:
//!
//! 1. Have the use feature-gate the proc-macro use itself, like this:
//!
//!    ```
//!    #[cfg_attr(feature = "native", derive(Clone))]
//!    struct MyStruct;
//!    ```
//!
//! 2. Modify the proc-macro to generate feature-gated code, resulting in more
//!    typical derive's, like this:
//!
//!    ```
//!    #[derive(Clone)]
//!    struct MyStruct;
//!    ```
//!
//! Option (1) is more verbose, whereas option (2) is more concise but requires
//! hard-coding the feature name.
//!
//! ## The `const _: () = {};` trick
//!
//! The `const _: () = {};` trick is a simple, yet effective way of creating a
//! new scope for you to generate code into, without worrying about
//! polluting the parent module.
//!
//! The key factor to consider here is that you won't be able to re-export
//! anything defined inside the scope that you define this way, so this trick is
//! appropriate when your proc-macro is e.g. implementing a trait, and not when
//! it's defining e.g. a new `struct` definition.
//!
//! Here's an example:
//!
//! ```
//! pub struct Foo(u32);
//!
//! const _: () = {
//!     use std::marker::PhantomData;
//!     use std::string::ToString;
//!
//!     #[automatically_derived]
//!     impl ToString for Foo {
//!         fn to_string(&self) -> String {
//!             format!("{}", self.0)
//!         }
//!     }
//! };
//! ```
//!
//! This trick is used by `serde` and other popular crates:
//! <https://github.com/serde-rs/serde/blob/3202a6858a2802b5aba2fa5cf3ec8f203408db74/serde_derive/src/dummy.rs#L15-L22>.
//!
//! ## Helper crates
//!
//! There's some incredible crates out there that can help tons when writing
//! even moderately complex proc-macros. If you're not familiar with the
//! proc-macro ecosystem, take some time to go through these lists and read the
//! descriptions of major crates to see if any of them can make your life
//! easier:
//!
//! - <https://lib.rs/development-tools/procedural-macro-helpers?sort=popular>
//! - <https://github.com/dtolnay/proc-macro-workshop>
//! - <https://old.reddit.com/r/rust/comments/16kdb3a/best_ways_to_learn_how_to_build_procedural_macros/>

// This crate is `missing_docs` because it is not intended to be used directly,
// but only through the re-exports in `sov_modules_api`. All re-exports are
// documented there.
#![allow(missing_docs)]

#[cfg(feature = "native")]
mod cli_parser;
mod common;
mod compile_manifest_constants;
mod dispatch;
mod event;
mod expand_macro;
mod manifest;
mod module_info;
mod rest;
#[cfg(feature = "native")]
mod rpc;

#[cfg(any(
    feature = "bench",
    all(feature = "gas-constant-estimation", feature = "native")
))]
mod metrics;

use compile_manifest_constants::{make_const_value, ConfigValueInput};
use dispatch::dispatch_call::DispatchCallMacro;
use dispatch::genesis::GenesisMacro;
use dispatch::hooks::HooksMacro;
use dispatch::message_codec::MessageCodec;
use event::EventMacro;
use proc_macro::TokenStream;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, DeriveInput, Path, Token};

// Inputs to the [`config_value`](crate::config_value) proc-macro.
/// Returns the name of the function that invoked the proc-macro.
// Shamelessly copy-pasted from <https://stackoverflow.com/a/40234666/5148606>.
macro_rules! fn_name {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        // We wouldn't want to crash if something goes wrong here (that would be
        // very confusing!).
        name.strip_suffix("::f")
            .unwrap_or("UNKNOWN")
            .split("::")
            .last()
            .unwrap_or("UNKNOWN")
    }};
}

#[proc_macro_derive(
    ModuleInfo,
    attributes(module_info, state, module, kernel_module, id, gas, phantom)
)]
pub fn module_info(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);

    handle_macro_error_and_expand(fn_name!(), module_info::derive_module_info(&input))
}

#[proc_macro_derive(Genesis, attributes(genesis))]
pub fn genesis(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let genesis_macro = GenesisMacro::new("Genesis");

    handle_macro_error_and_expand(fn_name!(), genesis_macro.derive_genesis(input))
}

#[proc_macro_derive(Hooks)]
pub fn hooks(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let hooks_macro = HooksMacro::new("Hooks");

    handle_macro_error_and_expand(fn_name!(), hooks_macro.derive_hooks(input))
}

#[proc_macro_derive(DispatchCall, attributes(dispatch_call))]
pub fn dispatch_call(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let call_macro = DispatchCallMacro::new("Call");

    handle_macro_error_and_expand(fn_name!(), call_macro.derive_dispatch_call(input))
}

#[proc_macro_derive(Event, attributes(event))]
pub fn event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let event_macro = EventMacro::new("Event");

    handle_macro_error_and_expand(fn_name!(), event_macro.derive_event_enum(input))
}

#[proc_macro_derive(UniversalWallet, attributes(sov_wallet, universal_wallet))]
pub fn derive_wallet(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let result = crate::common::get_derived_struct_subattr::<syn::TypePath>(
        &input,
        "universal_wallet",
        "sov_modules_api_path",
        syn::parse_quote! { sov_modules_api },
    )
    .and_then(|sov_api_type_path| {
        sov_universal_wallet_macro_helpers::schema::derive(
            input,
            Some(sov_api_type_path),
            syn::parse_quote! { sov_modules_api::macros::UniversalWallet },
        )
    });

    handle_macro_error_and_expand(fn_name!(), result.map(Into::into))
}

/// Adds encoding functionality to the underlying type.
#[proc_macro_derive(MessageCodec)]
pub fn codec(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let codec_macro = MessageCodec::new("MessageCodec");

    handle_macro_error_and_expand(fn_name!(), codec_macro.derive_message_codec(input))
}

/// A convenience macro that adds serialization-related derives and attributes.
/// It takes `Borsh` and/or `Serde` as arguments to generate the appropriate implementations.
///
/// ## Example
/// ```rust,ignore
/// #[serialize(Borsh, Serde)]
/// #[derive(Debug, PartialEq, Eq, Clone)]
/// pub enum MyMessage {
///   // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn serialize(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match Punctuated::<Path, Token![,]>::parse_terminated.parse(attr) {
        Ok(args) => args,
        Err(err) => return err.to_compile_error().into(),
    };
    let ast = parse_macro_input!(item as DeriveInput);

    let mut borsh_derives = quote::quote! {};
    let mut serde_derives = quote::quote! {};

    for arg in args {
        if arg.is_ident("Borsh") {
            borsh_derives =
                quote::quote! { #[derive(borsh::BorshDeserialize, borsh::BorshSerialize)] };
        } else if arg.is_ident("Serde") {
            serde_derives = quote::quote! {
                #[derive(serde::Serialize, serde::Deserialize)]
            };
        } else if let Some(ident) = arg.get_ident() {
            let err_msg = format!("Unsupported serialization format: `{ident}`. Supported formats are `Borsh` and `Serde`.");
            return syn::Error::new_spanned(arg, err_msg)
                .to_compile_error()
                .into();
        }
    }

    let expanded = quote::quote! {
        #borsh_derives
        #serde_derives
        #ast
    };

    expanded.into()
}

#[proc_macro]
pub fn config_value(item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as ConfigValueInput);
    handle_macro_error_and_expand(fn_name!(), make_const_value(&input).map(Into::into))
}

/// Like [`config_value!`], but for use within the `sov_modules_api` crate only.
#[proc_macro]
pub fn config_value_private(item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as ConfigValueInput);
    let tokens = make_const_value(&input)
        .map(|tokens| {
            quote::quote!({
                use crate as sov_modules_api;
                #tokens
            })
        })
        .map(Into::into);

    handle_macro_error_and_expand(fn_name!(), tokens)
}

#[cfg(any(feature = "native", feature = "bench"))]
struct AttributeArgs(syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>);

#[cfg(any(feature = "native", feature = "bench"))]
impl syn::parse::Parse for AttributeArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(AttributeArgs(
            syn::punctuated::Punctuated::parse_terminated(input)?,
        ))
    }
}

#[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
#[proc_macro_attribute]
pub fn track_gas_constants_usage(attr: TokenStream, item: TokenStream) -> TokenStream {
    use std::collections::HashSet;

    let attr_contents = parse_macro_input!(attr as AttributeArgs);

    let attr_inputs = attr_contents
        .0
        .into_iter()
        .filter_map(|meta| match meta {
            syn::Meta::Path(path) => path.get_ident().cloned(),
            _ => panic!(
                "Only path meta items are supported for the `track_gas_constants_usage` macro attribute"
            ),
        })
        .collect::<HashSet<_>>();

    metrics::wrap_function_with(metrics::gas_estimation::const_tracker, item, attr_inputs)
        .unwrap_or_else(|err| err.to_compile_error().into())
}

/// This macro is used to annotate functions that we want to track the number of riscV cycles being
/// generated inside the VM. The purpose of the this macro is to measure how many cycles a rust
/// function takes because prover time is directly proportional to the number of riscv cycles
/// generated. We are using the `cycle-count` module defined in `sov-metrics` to interface with zkvm
/// specific methods.
///
/// For instance, for RISC0, it does this by making use of a risc0 provided function
/// ```rust,ignore
/// risc0_zkvm_platform::syscall::sys_cycle_count
/// ```
///
/// For SP1, we are using the `sp1-lib` crate that provides file descriptors to communicate with the zk-guest.
///
/// The macro essentially generates new function with the same name by wrapping the body with calls to `get_cycle_counts`
/// at the beginning and end of the function, subtracting it and then emitting it out using `report_cycle_count`
#[cfg(feature = "bench")]
#[proc_macro_attribute]
pub fn cycle_tracker(attr: TokenStream, item: TokenStream) -> TokenStream {
    use std::collections::HashSet;

    let attr_contents = parse_macro_input!(attr as AttributeArgs);

    let attr_inputs = attr_contents
        .0
        .into_iter()
        .filter_map(|meta| match meta {
            syn::Meta::Path(path) => path.get_ident().cloned(),
            _ => {
                panic!("Only path meta items are supported for the `cycle_tracker` macro attribute")
            }
        })
        .collect::<HashSet<_>>();

    metrics::wrap_function_with(metrics::zk::guest_metrics, item, attr_inputs)
        .unwrap_or_else(|err| err.to_compile_error().into())
}

#[proc_macro_attribute]
#[cfg(feature = "native")]
pub fn rpc_gen(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_contents = parse_macro_input!(attr as AttributeArgs);
    let input = parse_macro_input!(item as syn::ItemImpl);
    handle_macro_error_and_expand(
        fn_name!(),
        rpc::rpc_gen(attr_contents.0.into_iter().collect(), input).map(Into::into),
    )
}

#[cfg(feature = "native")]
#[proc_macro_attribute]
pub fn expose_rpc(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let original = input.clone();
    let input = parse_macro_input!(input);
    handle_macro_error_and_expand(fn_name!(), rpc::expose_rpc("Expose", original, input))
}

#[proc_macro_derive(RuntimeRestApi, attributes(rest_api))]
pub fn runtime_metadata_rest_api(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    handle_macro_error_and_expand(fn_name!(), rest::runtime::derive(&input).map(Into::into))
}

/// Derives REST API endpoints for module state items.
///
/// ## Note
/// Generation of state item endpoints can fail silently, a common cause for this is missing
/// serde bounds for items stored in state.
///
/// If the stored item references the `Spec` generic you may need to add a serde bound like so:
/// ```ignore
/// #[serde(bound = "S: Spec")]
/// pub struct MyState<S: Spec> {
///   item: MyItem<S>
/// }
/// ```
#[proc_macro_derive(ModuleRestApi, attributes(rest_api))]
pub fn module_metadata_rest_api(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    handle_macro_error_and_expand(fn_name!(), rest::module::derive(&input).map(Into::into))
}

#[cfg(feature = "native")]
#[proc_macro_derive(CliWallet, attributes(cli_skip))]
pub fn cli_parser(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    handle_macro_error_and_expand(fn_name!(), cli_parser::derive_cli_wallet("Cmd", input))
}

fn expand_code(macro_name: &str, input: TokenStream) -> TokenStream {
    if std::env::var_os("SOV_EXPAND_PROC_MACROS").is_some() {
        expand_macro::expand_to_file(input.clone(), macro_name).unwrap_or_else(|err| {
            eprintln!("Failed to write to file proc-macro generated code: {err:?}");
            input
        })
    } else {
        input
    }
}

fn handle_macro_error_and_expand(
    macro_name: &str,
    result: syn::Result<TokenStream>,
) -> TokenStream {
    expand_code(
        macro_name,
        result.unwrap_or_else(|err| err.to_compile_error().into()),
    )
}
