use super::common::{parse_generic_params, StructDef, StructFieldExtractor, CALL, QUERY};
use proc_macro2::TokenStream;
use quote::format_ident;
use syn::DeriveInput;

impl<'a> StructDef<'a> {
    fn create_message_codec(&self) -> TokenStream {
        let call_enum = self.enum_ident(CALL);
        let query_enum = self.enum_ident(QUERY);
        let ty_generics = &self.type_generics;

        let fns = self.fields.iter().map(|field| {
            let variant = &field.ident;
            let ty = &field.ty;

            let fn_call_name = format_ident!("encode_{}_call", &field.ident);
            let fn_query_name = format_ident!("encode_{}_query", &field.ident);

            // Creates functions like:
            //  encode_*module_name*_call(data: ..) -> Vec<u8>
            //  encode_*module_name*_query(data: ..) -> Vec<u8>
            quote::quote! {
                pub (crate) fn #fn_call_name(data: <#ty as sov_modules_api::Module>::CallMessage) -> std::vec::Vec<u8> {
                    let call = #call_enum::<C>::#variant(data);
                    ::borsh::BorshSerialize::try_to_vec(&call)
                        .expect("CallMessage in memory serialization failed, probably OOM")
                }

                pub (crate) fn #fn_query_name(data: <#ty as sov_modules_api::Module>::QueryMessage)-> std::vec::Vec<u8>{
                    let query = #query_enum::<C>::#variant(data);
                    ::borsh::BorshSerialize::try_to_vec(&query)
                        .expect("QueryMessage in memory serialization failed, probably OOM")
                }
            }
        });

        let original_ident = &self.ident;
        let impl_generics = &self.impl_generics;
        let where_clause = self.where_clause;

        // Adds decoding functionality to the underlying type and
        // hides auto generated types behind impl DispatchQuery & impl DispatchCall.
        quote::quote! {
            impl #impl_generics #original_ident #ty_generics #where_clause {
                #(#fns)*
            }
        }
    }
}

pub(crate) struct MessageCodec {
    field_extractor: StructFieldExtractor,
}

impl MessageCodec {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn derive_message_codec(
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

        Ok(struct_def.create_message_codec().into())
    }
}
