use proc_macro2::{Span, TokenStream};
use syn::DeriveInput;

use crate::common::{
    get_generics_type_param, pascal_case_ident, StructDef, StructFieldExtractor, CALL,
};

impl StructDef<'_> {
    fn create_message_codec(&self) -> TokenStream {
        let original_ident = &self.ident;
        let call_enum = self.enum_ident(CALL);
        let ty_generics = &self.type_generics;
        let impl_generics = &self.impl_generics;
        let where_clause = &self.where_clause;

        let fns = self.fields.iter().map(|field| {
            let variant = pascal_case_ident(&field.ident);
            let ty = &field.ty;

            let decode_doc = format!("Encodes {} call message to {}Call.", field.ident, original_ident);

            quote::quote! {
            impl #impl_generics sov_modules_api::EncodeCall<#ty> for #original_ident #ty_generics #where_clause {
                #[doc = #decode_doc]
                fn to_decodable(data: <#ty as sov_modules_api::Module>::CallMessage) -> <Self as ::sov_modules_api::DispatchCall>::Decodable {
                    #call_enum:: #ty_generics ::#variant(data)
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
    ) -> syn::Result<proc_macro::TokenStream> {
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
