use proc_macro::TokenStream;
use quote::quote;
use syn::DeriveInput;

use crate::common::StructFieldExtractor;

pub fn expose_rpc(
    proc_macro_name: &'static str,
    original: TokenStream,
    input: DeriveInput,
) -> syn::Result<TokenStream> {
    let field_extractor = StructFieldExtractor::new(proc_macro_name);

    let DeriveInput {
        data,
        generics,
        ident: input_ident,
        ..
    } = input;

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let merge_operations = field_extractor
        .get_fields_from_struct(&data)?
        .iter()
        .map(|field| {
            let attrs = &field.attrs;
            let module_ident = &field.ident;

            quote! {
                #(#attrs)*
                rpc_module
                    .merge((&runtime.#module_ident).rpc_methods(api_state.clone()))
                    .unwrap();
            }
        })
        .collect::<Vec<_>>();

    let runtime_type = quote! {
        #input_ident #ty_generics
    };

    let fn_get_rpc_methods: proc_macro2::TokenStream = quote! {
        /// Returns a [`::sov_modules_api::prelude::jsonrpsee::RpcModule`] with all the RPC methods
        /// exposed by the runtime.
        pub fn get_rpc_methods #impl_generics (
            api_state: ::sov_modules_api::rest::ApiState<<#runtime_type as sov_modules_api::module::DispatchCall>::Spec>,
        ) -> ::sov_modules_api::prelude::jsonrpsee::RpcModule<()> #where_clause {
            // The attributes from merge operations may generate "unused doc
            // comment" warnings. Just to be safe, we ignore absolutely all
            // warnings.
            #![allow(warnings)]

            use sov_modules_api::__rpc_macros_private::ModuleWithRpcServer;

            let runtime = <#runtime_type as ::core::default::Default>::default();
            let mut rpc_module = ::sov_modules_api::prelude::jsonrpsee::RpcModule::new(());

            #(#merge_operations)*
            rpc_module
        }
    };

    let mut tokens = TokenStream::new();

    tokens.extend(original);
    tokens.extend(TokenStream::from(fn_get_rpc_methods));

    Ok(tokens)
}
