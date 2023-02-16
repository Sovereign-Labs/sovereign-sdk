mod extend_module;
use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Macro generating a `_new` method and prefixes for the underlying sov-module.
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
///  let test_struct = test_module::TestModule::<SomeContext>::_new(some_storage);
///  let prefix1 = test_struct.test_state1.prefix;
/// ````
/// ## Attributes
///
///  * `state` - attribute for state members
///  * `module` - attribute for module members
#[proc_macro_derive(ModuleInfo, attributes(state, module))]
pub fn module(input: TokenStream) -> TokenStream {
    //eprintln!("{input:?}");

    let input = parse_macro_input!(input);

    match extend_module::module(input) {
        Ok(ok) => ok,
        Err(err) => err.to_compile_error().into(),
    }
}
