use proc_macro2::Span;
use syn::DeriveInput;

use super::common::{
    get_generics_type_param, get_serialization_attrs, StructDef, StructFieldExtractor,
};

pub(crate) const EVENT: &str = "Event";

pub(crate) struct EventMacro {
    field_extractor: StructFieldExtractor,
}

impl<'a> StructDef<'a> {
    fn create_event_enum_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = &field.ident;
                let ty = &field.ty;

                quote::quote!(
                    #[doc = "Module event."]
                    #name(<#ty as ::sov_modules_api::Module>::Event),
                )
            })
            .collect()
    }
}

impl EventMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn derive_event_enum(
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

        let event_enum_legs = struct_def.create_event_enum_legs();
        let event_enum = struct_def.create_enum(&event_enum_legs, EVENT, &serialization_methods);

        Ok(quote::quote! {
            #[doc="This enum is generated from the underlying Runtime, the variants correspond to events from the relevant modules"]
            #event_enum
        }
            .into())
    }
}
