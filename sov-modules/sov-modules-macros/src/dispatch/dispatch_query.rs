use super::common::{
    get_serialization_attrs, parse_generic_params, StructDef, StructFieldExtractor, QUERY,
    QUERY_RESPONSE,
};
use proc_macro2::Ident;
use quote::format_ident;
use syn::DeriveInput;

/// A handy function that gpt4 generated to convert snake-case identifiers to camel-case
pub(crate) fn convert_snake_case_to_upper_camel_case(ident: &Ident) -> Ident {
    let snake_case_str = ident.to_string();
    let mut upper_camel_case_str = String::new();
    let mut capitalize_next = true;

    for ch in snake_case_str.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            upper_camel_case_str.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            upper_camel_case_str.push(ch);
        }
    }

    format_ident!("{}", upper_camel_case_str)
}

impl<'a> StructDef<'a> {
    fn create_query_message_enum_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = convert_snake_case_to_upper_camel_case(&field.ident);
                let ty = &field.ty;

                quote::quote!(
                    #name(<#ty as sov_modules_api::Module>::QueryMessage),
                )
            })
            .collect()
    }

    fn create_query_response_enum_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = convert_snake_case_to_upper_camel_case(&field.ident);
                let ty = &field.ty;

                quote::quote!(
                    #name(<#ty as sov_modules_api::Module>::QueryResponse),
                )
            })
            .collect()
    }

    /// Implements `sov_modules_api::DispatchQuery` for the enumeration created by `create_enum`.
    fn create_query_dispatch(&self) -> proc_macro2::TokenStream {
        let query_enum_ident = &self.enum_ident(QUERY);
        let query_response_enum_ident = &self.enum_ident(QUERY_RESPONSE);
        let type_generics = &self.type_generics;

        let match_legs = self.fields.iter().map(|field| {
            let field_name = &field.ident;
            let variant_name = convert_snake_case_to_upper_camel_case(&field.ident);

            quote::quote!(
                #query_enum_ident::#variant_name(message)=>{
                    #query_response_enum_ident::#variant_name(sov_modules_api::Module::query(&self.#field_name, message, working_set))
                },
            )
        });

        let ident = &self.ident;
        let impl_generics = &self.impl_generics;
        let where_clause = self.where_clause;
        let generic_param = self.generic_param;
        let ty_generics = &self.type_generics;

        quote::quote! {
            impl #impl_generics sov_modules_api::DispatchQuery for #ident #type_generics #where_clause{
                type Context = #generic_param;
                type Decodable = #query_enum_ident #ty_generics;
                type QueryResponse = #query_response_enum_ident #ty_generics;

                fn decode_query(serialized_message: &[u8]) -> core::result::Result<Self::Decodable, std::io::Error> {
                    let mut data = std::io::Cursor::new(serialized_message);
                    <#query_enum_ident #ty_generics as sovereign_sdk::serial::Decode>::decode(&mut data)
                }

                fn dispatch_query(
                    &self,
                    decodable: Self::Decodable,
                    working_set: &mut sov_state::WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>
                ) -> Self::QueryResponse {

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

        let query_enum_legs = struct_def.create_query_message_enum_legs();
        let query_response_enum_legs = struct_def.create_query_response_enum_legs();
        let mut query_enum_attrs = serialization_methods.clone();
        query_enum_attrs.push(quote::quote!(PartialEq));
        let query_enum = struct_def.create_enum(&query_enum_legs, QUERY, &query_enum_attrs);
        let create_dispatch_impl = struct_def.create_query_dispatch();

        let query_response_enum = struct_def.create_enum(
            &query_response_enum_legs,
            QUERY_RESPONSE,
            &serialization_methods,
        );

        Ok(quote::quote! {
            #query_enum

            #query_response_enum

            #create_dispatch_impl
        }
        .into())
    }
}
