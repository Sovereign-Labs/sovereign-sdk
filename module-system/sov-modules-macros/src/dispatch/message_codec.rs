use proc_macro2::{Span, TokenStream};
use syn::DeriveInput;

use crate::common::{get_generics_type_param, StructDef, StructFieldExtractor, CALL};

impl<'a> StructDef<'a> {
    fn create_message_codec(&self) -> TokenStream {
        let original_ident = &self.ident;
        let call_enum = self.enum_ident(CALL);
        let ty_generics = &self.type_generics;
        let impl_generics = &self.impl_generics;
        let where_clause = &self.where_clause;

        let fns = self.fields.iter().map(|field| {
            let variant = &field.ident;
            let ty = &field.ty;

            let call_doc = format!("Encodes {} call message.", field.ident);

            // Creates functions like:
            //  encode_*module_name*_call(data: ..) -> Vec<u8>
            //  encode_*module_name*_query(data: ..) -> Vec<u8>
            quote::quote! {
            impl #impl_generics sov_modules_api::EncodeCall<#ty> for #original_ident #ty_generics #where_clause {
                #[doc = #call_doc]
                fn encode_call(data: <#ty as sov_modules_api::Module>::CallMessage) -> std::vec::Vec<u8> {
                    let call = #call_enum:: #ty_generics ::#variant(data);
                    ::borsh::BorshSerialize::try_to_vec(&call).unwrap()
                }
            }
            }
        });

        // Adds decoding functionality to the underlying type and
        // hides auto generated types behind impl DispatchCall.
        quote::quote! {
            #(#fns)*
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

        let generic_param = get_generics_type_param(&generics, Span::call_site())?;

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
