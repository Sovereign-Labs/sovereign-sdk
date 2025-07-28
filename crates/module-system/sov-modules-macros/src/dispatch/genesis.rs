use proc_macro2::Span;
use syn::{DeriveInput, ImplGenerics, TypeGenerics, WhereClause};

use crate::common::{
    get_derived_enum_attrs, get_generics_type_param, StructFieldExtractor, StructNamedField,
};

pub(crate) struct GenesisMacro {
    field_extractor: StructFieldExtractor,
}

impl GenesisMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn derive_genesis(
        &self,
        input: DeriveInput,
    ) -> syn::Result<proc_macro::TokenStream> {
        let default_attrs = vec![quote::quote! {
            #[derive(Clone, ::serde::Deserialize, ::serde::Serialize)]
        }];
        let config_attributes = get_derived_enum_attrs("genesis", &input, default_attrs)?;

        let DeriveInput {
            data,
            ident,
            generics,
            ..
        } = input;

        let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

        let fields = self.field_extractor.get_fields_from_struct(&data)?;
        let generic_param = get_generics_type_param(&generics, Span::call_site())?;
        let genesis_config = Self::make_genesis_config(
            &fields,
            &impl_generics,
            &type_generics,
            where_clause,
            &config_attributes,
        );
        let genesis_fn_body = Self::make_genesis_fn_body(&fields);

        // Implements the Genesis trait
        Ok(quote::quote! {
            #genesis_config

            impl #impl_generics ::sov_modules_api::Genesis for #ident #type_generics #where_clause {
                type Spec = #generic_param;
                type Config = GenesisConfig #type_generics;

                fn genesis(&mut self,
                    genesis_rollup_header: &<<Self::Spec as ::sov_modules_api::Spec>::Da as ::sov_modules_api::DaSpec>::BlockHeader,
                    config: &Self::Config,
                    state: &mut impl ::sov_modules_api::GenesisState<<Self as ::sov_modules_api::Genesis>::Spec>) -> core::result::Result<(), sov_modules_api::Error> {
                    #genesis_fn_body
                    Ok(())
                }
            }
        }
        .into())
    }

    fn make_genesis_fn_body(fields: &[StructNamedField]) -> proc_macro2::TokenStream {
        let idents = fields.iter().enumerate().map(|(i, field)| {
            let ident = &field.ident;

            quote::quote! {
                (&self.#ident, #i)
            }
        });

        let matches = fields.iter().enumerate().map(|(i, field)| {
            let ident = &field.ident;

            quote::quote! {
                #i => ::sov_modules_api::Genesis::genesis(&mut self.#ident, genesis_rollup_header, &config.#ident, state),
            }
        });

        quote::quote! {
                let modules: ::std::vec::Vec<(&dyn ::sov_modules_api::ModuleInfo<Spec = <Self as sov_modules_api::Genesis>::Spec>, usize)> = ::std::vec![#(#idents),*];
                let sorted_modules = ::sov_modules_api::sort_values_by_modules_dependencies(modules)?;
                for module in sorted_modules {
                     match module {
                         #(#matches)*
                         _ => Err(::sov_modules_api::Error::ModuleError(::anyhow::Error::msg(format!("Module not found. Please verify that the module is included in the Runtime: {:?}", module)))),
                     }?
                }
        }
    }

    fn make_genesis_config(
        fields: &[StructNamedField],
        impl_generics: &ImplGenerics,
        type_generics: &TypeGenerics,
        where_clause: Option<&WhereClause>,
        attributes: &[proc_macro2::TokenStream],
    ) -> proc_macro2::TokenStream {
        let field_names = fields.iter().map(|field| &field.ident);

        let fields: &Vec<proc_macro2::TokenStream> = &fields
            .iter()
            .map(|field| {
                let name = &field.ident;
                let ty = &field.ty;

                quote::quote! {
                    #name: <#ty as sov_modules_api::Module>::Config,
                }
            })
            .collect();

        quote::quote! {
            #[doc = "Initial configuration for the rollup."]
            #(#attributes)*
            pub struct GenesisConfig #impl_generics #where_clause{
                #(#[doc = "Module configuration"] pub #fields)*
            }

            impl #impl_generics GenesisConfig #type_generics #where_clause {
                #[doc = "GenesisConfig constructor."]
                #[allow(clippy::too_many_arguments)]
                pub fn new(#(#fields)*) -> Self {
                    Self {
                        #(#field_names),*
                    }
                }
            }
        }
    }
}
