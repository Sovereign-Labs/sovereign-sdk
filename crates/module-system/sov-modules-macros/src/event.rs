use proc_macro2::Span;
use syn::DeriveInput;

use super::common::{
    get_derived_enum_attrs, get_generics_type_param, StructDef, StructFieldExtractor,
};
use crate::common::pascal_case_ident;

pub(crate) const EVENT: &str = "Event";

pub(crate) struct EventMacro {
    field_extractor: StructFieldExtractor,
}

impl StructDef<'_> {
    fn create_event_enum_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = pascal_case_ident(&field.ident);
                let ty = &field.ty;

                quote::quote!(
                    #[doc = "Module event."]
                    #name(<#ty as ::sov_modules_api::Module>::Event),
                )
            })
            .collect()
    }

    pub(crate) fn enum_to_inner_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = pascal_case_ident(&field.ident);
                quote::quote!(
                    Self:: #name(inner) => inner,
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
    ) -> syn::Result<proc_macro::TokenStream> {
        let default_event_attrs = vec![
            quote::quote! {
                #[
                    derive(
                        borsh::BorshDeserialize,
                        borsh::BorshSerialize,
                        serde::Serialize,
                        serde::Deserialize,
                        Clone,
                        Debug,
                        PartialEq,
                        sov_modules_api::prelude::schemars::JsonSchema,
                        sov_modules_api::prelude::strum::EnumDiscriminants,
                        sov_modules_api::prelude::strum::VariantNames,
                        sov_modules_api::prelude::strum::EnumTryAs,
                        sov_modules_api::prelude::strum::IntoStaticStr,
                        sov_modules_api::prelude::strum::AsRefStr,
                    )
                ]
            },
            quote::quote! {
                #[serde(untagged, bound = "", rename_all = "snake_case")]
            },
            quote::quote! {
                #[strum_discriminants(derive(
                    sov_modules_api::prelude::strum::VariantNames,
                    sov_modules_api::prelude::strum::VariantArray,
                    sov_modules_api::prelude::strum::EnumString,
                    sov_modules_api::prelude::strum::IntoStaticStr,
                    sov_modules_api::prelude::strum::AsRefStr,
                ))]
            },
        ];
        let enum_attributes = get_derived_enum_attrs("event", &input, default_event_attrs)?;
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
        let enum_to_inner_legs = struct_def.enum_to_inner_legs();
        let event_enum = struct_def.create_enum(
            &event_enum_legs,
            &enum_to_inner_legs,
            EVENT,
            &enum_attributes,
        );

        let event_cases = struct_def.fields.iter().map(|field| {
            let name = pascal_case_ident(&field.ident);
            let module_ty = &field.ty;

            quote::quote! {
        _ if event.type_id() == &core::any::TypeId::of::<<#module_ty as ::sov_modules_api::Module>::Event>() => {
            event.downcast::<<#module_ty as ::sov_modules_api::Module>::Event>()
                  .map(|boxed_event| Self::RuntimeEvent::#name(boxed_event))
        }
    }
        });

        let impl_generics = &struct_def.impl_generics;
        let type_generics = &struct_def.type_generics;
        let ident_name = &struct_def.ident;
        let event_enum_name = struct_def.enum_ident(EVENT);

        let from_event_cases = struct_def.fields.iter().map(|field| {
            let variant_name = pascal_case_ident(&field.ident);
            let module_name = pascal_case_ident(&field.ident);

            quote::quote! {
                #event_enum_name::#variant_name(ref event) => {
                    stringify!(#module_name)
                }
            }
        });

        let impl_runtime_event_module_name = quote::quote! {
            #[automatically_derived]
            impl #impl_generics ::sov_modules_api::EventModuleName for #event_enum_name #type_generics #where_clause {
                fn module_name(&self) -> &'static str {
                    match self {
                        #(#from_event_cases),*
                    }
                }
            }
        };

        let impl_runtime_event_processor = quote::quote! {
            #[automatically_derived]
            impl #impl_generics ::sov_modules_api::RuntimeEventProcessor for #ident_name #type_generics #where_clause {
                type RuntimeEvent = #event_enum_name #type_generics;

                fn convert_to_runtime_event(
                    event: ::sov_modules_api::TypeErasedEvent
                ) -> Option<Self::RuntimeEvent> {
                    match event.type_id() {
                        #(#event_cases)*
                        _ => None,
                    }
                }
            }
        };

        Ok(
            quote::quote! {

            mod __generated_event_impl {
                #![allow(missing_docs)]
                use super::*;

                #[doc="This enum is generated from the underlying Runtime, the variants correspond to events from the relevant modules"]
                #event_enum

                #impl_runtime_event_processor

                #impl_runtime_event_module_name
            }
            pub use __generated_event_impl::*;
        }.into())
    }
}
