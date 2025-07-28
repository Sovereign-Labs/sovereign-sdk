use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(UniversalWallet, attributes(sov_wallet))]
pub fn derive_wallet(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let result = sov_universal_wallet_macro_helpers::schema::derive(
        input,
        Some(syn::parse_quote! { sov_rollup_interface }),
        syn::parse_quote! { sov_rollup_interface::sov_universal_wallet::UniversalWallet },
    );
    handle_macro_error_and_expand(result.map(Into::into))
}

// TODO: extract the expand_macro logic from sov_module_macros into... probably somewhere
// in crates/utils/ and allow it to be reused here
pub(crate) fn handle_macro_error_and_expand(result: syn::Result<TokenStream>) -> TokenStream {
    result.unwrap_or_else(|err| err.to_compile_error().into())
}
