mod dispatch;
mod module_info;
use dispatch::{
    dispatch_call::DispatchCallMacro, dispatch_query::DispatchQueryMacro, genesis::GenesisMacro,
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
///  let test_struct = <TestModule::<SomeContext> as sov_modules_api::ModuleInfo<SomeContext>>::new(some_storage);
///  let prefix1 = test_struct.test_state1.prefix;
/// ````
/// ## Attributes
///
///  * `state` - attribute for state members
///  * `module` - attribute for module members
#[proc_macro_derive(ModuleInfo, attributes(state, module))]
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
#[proc_macro_derive(DispatchCall)]
pub fn dispatch_call(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let call_macro = DispatchCallMacro::new("Call");

    handle_macro_error(call_macro.derive_dispatch_call(input))
}

/// Derives the `sov-modules-api::DispatchQuery` implementation for the underlying type.
#[proc_macro_derive(DispatchQuery)]
pub fn dispatch_query(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    let query_macro = DispatchQueryMacro::new("Query");

    handle_macro_error(query_macro.derive_dispatch_query(input))
}

fn handle_macro_error(result: Result<proc_macro::TokenStream, syn::Error>) -> TokenStream {
    match result {
        Ok(ok) => ok,
        Err(err) => err.to_compile_error().into(),
    }
}
