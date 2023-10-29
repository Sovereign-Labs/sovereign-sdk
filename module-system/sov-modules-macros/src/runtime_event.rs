use proc_macro2::Span;
use syn::DeriveInput;

use super::common::{
    get_generics_type_param, get_serialization_attrs, StructDef, StructFieldExtractor,
};

pub(crate) const EVENT: &str = "Event";

pub(crate) struct RuntimeEventMacro {
    field_extractor: StructFieldExtractor,
}

impl<'a> StructDef<'a> {
    fn create_event_enum_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = &field.ident;
                let ty = &field.ty;

                let doc: String = format!("An event emitted by the {} module", name);

                quote::quote!(
                    #[doc = #doc]
                    #name(<#ty as ::sov_modules_api::Module>::Event),
                )
            })
            .collect()
    }

    fn create_get_key_string_impl(&self) -> proc_macro2::TokenStream {
        let enum_ident = self.enum_ident(EVENT);

        let match_legs: Vec<proc_macro2::TokenStream> = self
            .fields
            .iter()
            .map(|field| {
                let module_name = &field.ident;
                let module_name_str = &field.ident.to_string();
                quote::quote!(
                    #enum_ident::#module_name(inner)=>{
                        format!("{}-{}", #module_name_str, inner.event_key())
                    },
                )
            })
            .collect();

        let impl_generics = &self.impl_generics;
        let enum_ident = self.enum_ident(EVENT);
        let where_clause = &self.where_clause;
        let ty_generics = &self.type_generics;

        quote::quote! {
            impl #impl_generics #enum_ident #ty_generics #where_clause {

                /// Returns a string that identifies both the module and the event type
                pub fn get_key_string(&self) -> String {
                    use ::sov_modules_api::Event as _;

                    match self {
                       #(#match_legs)*
                    }
                }
            }
        }
    }
}

impl RuntimeEventMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn derive_runtime_event(
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

        let enum_legs = struct_def.create_event_enum_legs();
        let event_enum = struct_def.create_enum(&enum_legs, EVENT, &serialization_methods);
        let get_key_string_impl = struct_def.create_get_key_string_impl();

        Ok(quote::quote! {
            #[doc="This enum is generated from the underlying Runtime, the variants correspond to events from the relevant modules"]
            #event_enum

            #get_key_string_impl
        }
            .into())
    }
}
