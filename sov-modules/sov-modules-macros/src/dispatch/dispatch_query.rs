use syn::DeriveInput;

use super::common::{parse_generic_params, StructDef, StructFieldExtractor, QUERY};

impl<'a> StructDef<'a> {
    fn create_query_enum_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = &field.ident;
                let ty = &field.ty;

                quote::quote!(
                    #name(<#ty as sov_modules_api::Module>::QueryMessage),
                )
            })
            .collect()
    }

    /// Implements `sov_modules_api::DispatchQuery` for the enumeration created by `create_enum`.
    fn create_query_dispatch(&self) -> proc_macro2::TokenStream {
        let enum_ident = &self.enum_ident(QUERY);
        let type_generics = &self.type_generics;

        let match_legs = self.fields.iter().map(|field| {
            let name = &field.ident;
            let ty = &field.ty;

            quote::quote!(
                #enum_ident::#name(message)=>{
                    let #name = <#ty as sov_modules_api::ModuleInfo::#type_generics>::new(storage.clone());
                    sov_modules_api::Module::query(&#name, message)
                },
            )
        });

        let impl_generics = &self.impl_generics;
        let where_clause = self.where_clause;
        let generic_param = self.generic_param;

        quote::quote! {
            impl #impl_generics sov_modules_api::DispatchQuery for #enum_ident #type_generics #where_clause{
                type Context = #generic_param;

                fn dispatch_query(
                    self,
                    storage: <Self::Context as sov_modules_api::Spec>::Storage
                ) -> sov_modules_api::QueryResponse {
                    match self{
                        #(#match_legs)*
                    }
                }
            }
        }
    }
}

pub(crate) struct DispatchQueryMacro {
    field_extractor: StructFieldExtractor,
}

impl DispatchQueryMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn derive_dispatch_query(
        &self,
        input: DeriveInput,
    ) -> Result<proc_macro::TokenStream, syn::Error> {
        let DeriveInput {
            data,
            ident,
            generics,
            ..
        } = input;

        let generic_param = parse_generic_params(&generics)?;

        let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
        let fields = self.field_extractor.get_fields_from_struct(&data)?;

        let struct_def = StructDef::new(
            ident,
            fields,
            impl_generics,
            type_generics,
            &generic_param,
            where_clause,
        );

        let query_enum_legs = struct_def.create_query_enum_legs();
        let query_enum = struct_def.create_enum(&query_enum_legs, QUERY);
        let create_dispatch_impl = struct_def.create_query_dispatch();

        Ok(quote::quote! {
            #query_enum

            #create_dispatch_impl
        }
        .into())
    }
}
