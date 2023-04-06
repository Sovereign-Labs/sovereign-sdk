use super::common::{parse_generic_params, StructDef, StructFieldExtractor, QUERY, get_serialization_attrs};
use syn::DeriveInput;

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

            quote::quote!(
                #enum_ident::#name(message)=>{
                    sov_modules_api::Module::query(&self.#name, message, working_set)
                },
            )
        });

        let ident = &self.ident;
        let impl_generics = &self.impl_generics;
        let where_clause = self.where_clause;
        let generic_param = self.generic_param;
        let ty_generics = &self.type_generics;
        let query_enum = self.enum_ident(QUERY);

        quote::quote! {
            impl #impl_generics sov_modules_api::DispatchQuery for #ident #type_generics #where_clause{
                type Context = #generic_param;
                type Decodable = #query_enum #ty_generics;

                fn decode_query(serialized_message: &[u8]) -> core::result::Result<Self::Decodable, std::io::Error> {
                    let mut data = std::io::Cursor::new(serialized_message);
                    <#query_enum #ty_generics as sovereign_sdk::serial::Decode>::decode(&mut data)
                }

                fn dispatch_query(
                    &self,
                    decodable: Self::Decodable,
                    working_set: &mut sov_state::WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>
                ) -> sov_modules_api::QueryResponse {

                    match decodable {
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
        let serialization_methods = get_serialization_attrs(&input)?;

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
        let query_enum = struct_def.create_enum(&query_enum_legs, QUERY, &serialization_methods);
        let create_dispatch_impl = struct_def.create_query_dispatch();

        Ok(quote::quote! {
            #query_enum

            #create_dispatch_impl
        }
        .into())
    }
}
