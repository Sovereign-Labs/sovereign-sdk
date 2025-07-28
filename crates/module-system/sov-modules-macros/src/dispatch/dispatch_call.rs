use proc_macro2::Span;
use syn::DeriveInput;

use crate::common::{
    get_derived_enum_attrs, get_generics_type_param, pascal_case_ident, StructDef,
    StructFieldExtractor, CALL,
};

impl StructDef<'_> {
    fn create_call_enum_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = pascal_case_ident(&field.ident);
                let ty = &field.ty;

                quote::quote!(
                    #[doc = "Module call message."]
                    #name(<#ty as ::sov_modules_api::Module>::CallMessage),
                )
            })
            .collect()
    }

    fn create_call_dispatch(&self) -> proc_macro2::TokenStream {
        let enum_ident = self.enum_ident(CALL);
        let discriminant_enum_ident = quote::format_ident!("{}Discriminants", enum_ident);
        let type_generics = &self.type_generics;

        let match_legs = self.fields.iter().map(|field| {
            let variant_ident = pascal_case_ident(&field.ident);
            let field_ident = &field.ident;

            quote::quote!(
                #enum_ident::#variant_ident(message) => {
                    ::std::result::Result::Ok(::sov_modules_api::Module::call(&mut self.#field_ident, message, context, state)?)
                },
            )
        });

        let match_legs_address = self.fields.iter().map(|field| {
            let variant_ident = pascal_case_ident(&field.ident);
            let field_ident = &field.ident;
            let ty = &field.ty;

            quote::quote!(
                #enum_ident::#variant_ident(message)=>{
                   <#ty as ::sov_modules_api::ModuleInfo>::id(&self.#field_ident)
                },
            )
        });

        let match_legs_info = self.fields.iter().map(|field| {
            let variant_ident = pascal_case_ident(&field.ident);
            let field_ident = &field.ident;

            quote::quote!(
                #discriminant_enum_ident::#variant_ident =>{
                   &self.#field_ident
                },
            )
        });

        let ident = &self.ident;
        let impl_generics = &self.impl_generics;
        let where_clause = self.where_clause;
        let generic_param = self.generic_param;
        let ty_generics = &self.type_generics;
        let call_enum = self.enum_ident(CALL);

        quote::quote! {
            impl #impl_generics ::sov_modules_api::DispatchCall for #ident #type_generics #where_clause {
                type Spec = #generic_param;
                type Decodable = #call_enum #ty_generics;

                fn encode(msg: &Self::Decodable) -> Vec<u8> {
                    ::borsh::to_vec(msg).expect("Serialization to vec is infallible")
                }


                fn dispatch_call<I: ::sov_modules_api::StateProvider<Self::Spec>>(
                    &mut self,
                    decodable: Self::Decodable,
                    state: &mut ::sov_modules_api::WorkingSet<Self::Spec, I>,
                    context: &::sov_modules_api::Context<Self::Spec>,
                ) -> ::core::result::Result<(), ::sov_modules_api::Error> {
                    ::sov_modules_api::prelude::tracing::trace!("Dispatching call: {:?}", decodable);

                    match decodable {
                        #(#match_legs)*
                    }

                }

                fn module_id(&self, decodable: &Self::Decodable) -> &::sov_modules_api::ModuleId {
                    match decodable {
                        #(#match_legs_address)*
                    }
                }

                fn module_info(
                    &self,
                    discriminant: <Self::Decodable as ::sov_modules_api::NestedEnumUtils>::Discriminants,
                ) -> &dyn ::sov_modules_api::ModuleInfo<Spec = Self::Spec> {
                    match discriminant {
                        #(#match_legs_info)*
                    }
                }

            }
        }
    }
}

pub(crate) struct DispatchCallMacro {
    field_extractor: StructFieldExtractor,
}

impl DispatchCallMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn derive_dispatch_call(
        &self,
        input: DeriveInput,
    ) -> syn::Result<proc_macro::TokenStream> {
        let default_attrs = vec![
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
                        Eq,
                        sov_modules_api::macros::UniversalWallet,
                        sov_modules_api::prelude::strum::EnumDiscriminants,
                        sov_modules_api::prelude::strum::VariantNames,
                        sov_modules_api::prelude::strum::EnumTryAs,
                        sov_modules_api::prelude::strum::IntoStaticStr,
                        sov_modules_api::prelude::strum::AsRefStr,
                        sov_modules_api::prelude::schemars::JsonSchema,
                    )
                ]
            },
            quote::quote! {
                #[sov_wallet(template_inherit)]
            },
            quote::quote! {
                #[serde(rename_all = "snake_case")]
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

        let enum_attributes = get_derived_enum_attrs("dispatch_call", &input, default_attrs)?;
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

        let call_enum_legs = struct_def.create_call_enum_legs();
        let enum_to_inner_legs = struct_def.enum_to_inner_legs();
        let call_enum =
            struct_def.create_enum(&call_enum_legs, &enum_to_inner_legs, CALL, &enum_attributes);

        let create_dispatch_impl = struct_def.create_call_dispatch();

        Ok(quote::quote! {
            mod __generated_dispatch_call_impl {
                #![allow(missing_docs)]
                use super::*;

                #[doc="This enum is generated from the underlying Runtime, the variants correspond to call messages from the relevant modules"]
                #call_enum

                #create_dispatch_impl
            }
            pub use __generated_dispatch_call_impl::*;
        }
        .into())
    }
}
