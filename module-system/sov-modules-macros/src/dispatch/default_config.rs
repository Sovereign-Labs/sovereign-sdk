use syn::DeriveInput;

use super::common::{StructFieldExtractor, StructNamedField};

pub(crate) struct DefaultConfigMacro {
    field_extractor: StructFieldExtractor,
}

impl DefaultConfigMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn derive_default_config(
        &self,
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
        let runtime_fn_body = Self::make_default_config_fn_body(&fields);

        // Implements the Default Config trait
        Ok(quote::quote! {
        impl #impl_generics Default for #ident #type_generics #where_clause {
            fn default() -> Self {
                use sov_modules_api::ModuleInfo;

                Self {
                #(#runtime_fn_body)*
                }
            }
        }
                }
        .into())
    }

    pub(crate) fn make_default_config_fn_body(
        fields: &Vec<StructNamedField>,
    ) -> Vec<proc_macro2::TokenStream> {
        fields
            .iter()
            .map(|field| {
                let name = &field.ident;
                let ty = &field.ty;

                quote::quote! {
                    #name: <#ty>::new(),
                }
            })
            .collect()
    }
}
