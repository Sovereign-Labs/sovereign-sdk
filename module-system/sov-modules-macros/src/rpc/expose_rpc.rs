use proc_macro2::{Ident, Span};
use quote::quote;
use syn::DeriveInput;

use crate::common::StructFieldExtractor;

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
        let DeriveInput {
            data,
            ident,
            generics,
            ..
        } = input;

        let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
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

            let module_ident = ty.path.segments.last().unwrap().clone().ident;

            let rpc_trait_ident =
                syn::Ident::new(&format!("{}RpcImpl", &module_ident), module_ident.span());

            let rpc_server_ident =
                syn::Ident::new(&format!("{}RpcServer", &module_ident), module_ident.span());

            let merge_operation = quote! {
                module
                    .merge(#rpc_server_ident::into_rpc(r.clone()))
                    .unwrap();
            };

            merge_operations.extend(merge_operation);

            let rpc_trait_impl = quote! {
                impl <C: ::sov_modules_api::Context> #rpc_trait_ident<C> for RpcStorage<C>{
                    fn get_working_set(&self) -> ::sov_state::WorkingSet<<C as ::sov_modules_api::Spec>::Storage>
                    {
                        ::sov_state::WorkingSet::new(self.storage.clone())
                    }
                }
            };
            rpc_trait_impls.extend(rpc_trait_impl);
        }

        let get_rpc_methods = quote! {
            pub fn get_rpc_methods(storage: <DefaultContext as ::sov_modules_api::Spec>::Storage) -> jsonrpsee::RpcModule<()> {
                let mut module = jsonrpsee::RpcModule::new(());
                let r = RpcStorage::<DefaultContext> {
                    storage: storage.clone(),
                };

                #merge_operations
                module
            }
        };

        let mut tokens = proc_macro::TokenStream::new();
        tokens.extend(original);
        let q: proc_macro::TokenStream = quote! {


            #get_rpc_methods

            #rpc_storage_struct

            #rpc_trait_impls
        }
        .into();

        tokens.extend(q);

        Ok(tokens)
    }
}
