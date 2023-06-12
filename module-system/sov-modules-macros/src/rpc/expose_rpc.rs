use proc_macro2::{Ident, Span};
use quote::quote;

pub(crate) fn expose_rpc(
    args: proc_macro2::TokenStream,
    input: syn::ItemImpl,
) -> Result<proc_macro::TokenStream, syn::Error> {
    let attrs = &input.attrs;

    let args: syn::Type = syn::parse2(args).expect("Expected a valid type list");

    let mut output_tokens = proc_macro2::TokenStream::new();

    let mut merge_operations = proc_macro2::TokenStream::new();

    let types = match args {
        syn::Type::Tuple(tuple) => tuple.elems,
        _ => panic!("Expected a tuple of types"),
    };

    let rpc_storage_struct = quote! {
        #[derive(Clone)]
        pub struct RpcStorage<C: Context> {
            pub storage: C::Storage
        }
    };

    let original_impl = quote! {
        #(#attrs)*
        #input
        #rpc_storage_struct
    };

    output_tokens.extend(original_impl);

    // will be replaced in the below loop
    // hack for now
    // TODO: handle context in a consistent way for module, runtime as well as state transition runner
    let mut last_context_type: Ident = Ident::new("Context", Span::call_site());

    for arg in types {
        let mut trait_type_path = match arg {
            syn::Type::Path(type_path) => type_path.clone(),
            _ => panic!("Expected a path type"),
        };

        let last_segment = trait_type_path.path.segments.last_mut().unwrap();
        let context_type = match last_segment.arguments {
            syn::PathArguments::AngleBracketed(ref args) => {
                match args
                    .args
                    .first()
                    .expect("Expected at least one type argument")
                {
                    syn::GenericArgument::Type(syn::Type::Path(ref type_path)) => {
                        // Assuming type path has only one segment
                        type_path
                            .path
                            .segments
                            .first()
                            .expect("Expected at least one segment")
                            .ident
                            .clone()
                    }
                    _ => panic!("Expected a type argument"),
                }
            }
            _ => panic!("Expected angle bracketed arguments"),
        };
        last_context_type = context_type.clone();

        let mut rpc_server_ident = last_segment.ident.clone();
        rpc_server_ident = syn::Ident::new(
            &format!("{}RpcServer", rpc_server_ident),
            rpc_server_ident.span(),
        );

        let merge_operation = quote! {
        module
            .merge(#rpc_server_ident::into_rpc(r.clone()))
            .unwrap();
        };
        merge_operations.extend(merge_operation);

        last_segment.ident = syn::Ident::new(
            &format!("{}RpcImpl", last_segment.ident),
            last_segment.ident.span(),
        );

        let output = quote! {

            impl #trait_type_path for RpcStorage<#context_type>
            {
                fn get_working_set(&self) -> ::sov_state::WorkingSet<<#context_type
                    as ::sov_modules_api::Spec>::Storage> {
                    ::sov_state::WorkingSet::new(self.storage.clone())
                }
            }
        };

        output_tokens.extend(output);
    }

    let create_rpc_tokens = quote! {
             pub fn get_rpc_methods(storj: <#last_context_type as ::sov_modules_api::Spec>::Storage) -> jsonrpsee::RpcModule<()> {
                let mut module = jsonrpsee::RpcModule::new(());
                let r = RpcStorage {
                    storage: storj.clone(),
                };

                #merge_operations
                module

            }
    };

    output_tokens.extend(create_rpc_tokens);

    Ok(output_tokens.into())
}
