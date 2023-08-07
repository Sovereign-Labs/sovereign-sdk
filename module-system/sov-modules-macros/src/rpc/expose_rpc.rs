use quote::quote;
use syn::DeriveInput;

use crate::common::{generics_for_field, StructFieldExtractor};

pub(crate) struct ExposeRpcMacro {
    field_extractor: StructFieldExtractor,
}

impl ExposeRpcMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn generate_rpc(
        &self,
        original: proc_macro::TokenStream,
        input: DeriveInput,
    ) -> Result<proc_macro::TokenStream, syn::Error> {
        let DeriveInput { data, generics, .. } = input;
        let (impl_generics, _ty_generics, where_clause) = generics.split_for_impl();
        let context_type = generics
            .params
            .iter()
            .find_map(|item| {
                if let syn::GenericParam::Type(type_param) = item {
                    Some(type_param.ident.clone())
                } else {
                    None
                }
            })
            .ok_or(syn::Error::new_spanned(
                &generics,
                "a runtime must be generic over a sov_modules_api::Context to derive CliWallet",
            ))?;

        let fields = self.field_extractor.get_fields_from_struct(&data)?;

        let rpc_storage_struct = quote! {
            #[derive(Clone)]
            struct RpcStorage<C: ::sov_modules_api::Context> {
                 storage: C::Storage
            }
        };

        let mut merge_operations = proc_macro2::TokenStream::new();
        let mut rpc_trait_impls = proc_macro2::TokenStream::new();

        for field in fields {
            let ty = match field.ty {
                syn::Type::Path(type_path) => type_path.clone(),
                _ => panic!("Expected a path type"),
            };

            let field_path_args = &ty
                .path
                .segments
                .last()
                .expect("A type path must have at least one segment")
                .arguments;
            let field_generics = generics_for_field(&generics, field_path_args);

            let module_ident = ty.path.segments.last().unwrap().clone().ident;

            let rpc_trait_ident =
                syn::Ident::new(&format!("{}RpcImpl", &module_ident), module_ident.span());

            let rpc_server_ident =
                syn::Ident::new(&format!("{}RpcServer", &module_ident), module_ident.span());

            let merge_operation = quote! {
                module
                    .merge(#rpc_server_ident:: #field_path_args ::into_rpc(r.clone()))
                    .unwrap();
            };

            merge_operations.extend(merge_operation);

            let rpc_trait_impl = quote! {
                impl #field_generics #rpc_trait_ident #field_path_args for RpcStorage<#context_type> {
                    fn get_working_set(&self) -> ::sov_state::WorkingSet<<#context_type as ::sov_modules_api::Spec>::Storage>
                    {
                        ::sov_state::WorkingSet::new(self.storage.clone())
                    }
                }
            };
            rpc_trait_impls.extend(rpc_trait_impl);
        }

        let get_rpc_methods: proc_macro2::TokenStream = quote! {
            /// Rpc methods getter.
            pub fn get_rpc_methods #impl_generics (storage: <#context_type as ::sov_modules_api::Spec>::Storage) -> jsonrpsee::RpcModule<()> #where_clause{
                let mut module = jsonrpsee::RpcModule::new(());
                let r = RpcStorage::<#context_type> {
                    storage: storage.clone(),
                };

                #merge_operations
                module
            }
        };

        let mut tokens = proc_macro::TokenStream::new();
        tokens.extend(original);

        let generated_from_runtime: proc_macro::TokenStream = quote! {
            #get_rpc_methods

            #rpc_storage_struct

            #rpc_trait_impls
        }
        .into();

        tokens.extend(generated_from_runtime);

        Ok(tokens)
    }
}
