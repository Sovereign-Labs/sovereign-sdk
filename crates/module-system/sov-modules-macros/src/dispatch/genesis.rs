use proc_macro2::Span;
use syn::{DeriveInput, ImplGenerics, TypeGenerics, WhereClause};

use crate::common::{
    get_derived_enum_attrs, get_generics_type_param, get_serde_bounds_str, StructFieldExtractor,
    StructNamedField,
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
        let serde_bounds_str = get_serde_bounds_str(&input.generics, "_genesis_de")?;
        let default_attrs = vec![
            quote::quote! {
                #[derive(Clone, ::serde::Deserialize, ::serde::Serialize)]
            },
            quote::quote! {
                #[serde(bound = #serde_bounds_str)]
            },
        ];
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
        )?;
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

        let discriminant_checks = fields.iter().map(|field| {
            let ident = &field.ident;
            quote::quote! {
                // Ensure that the user didn't accidentally set the same discriminant for multiple modules
                if !discriminants.insert(::sov_modules_api::ModuleInfo::discriminant(&self.#ident)) {
                    return Err(::sov_modules_api::Error::ModuleError(::anyhow::Error::msg(format!("Duplicate module discriminant for {}. Update your constants.toml to give each module a unique discriminant", std::any::type_name_of_val(&self.#ident)))));
                }
            }
        });

        quote::quote! {

                let modules: ::std::vec::Vec<(&dyn ::sov_modules_api::ModuleInfo<Spec = <Self as sov_modules_api::Genesis>::Spec>, usize)> = ::std::vec![#(#idents),*];
                let sorted_modules = ::sov_modules_api::sort_values_by_modules_dependencies(modules)?;
                let mut discriminants = ::std::collections::HashSet::with_capacity(sorted_modules.len());
                #(#discriminant_checks)*
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
    ) -> Result<proc_macro2::TokenStream, syn::Error> {
        let chain_state_field_name = fields.iter().find(|field| field.ident == "chain_state").ok_or_else(|| syn::Error::new(Span::call_site(), "Chain state field not found. The Genesis macro may only be used if your runtime contains the `sov_chain_state::ChainState` module with the name `chain_state`. If you need a custom chain name, reach out to the SDK developers for support."))?.ident.clone();
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

        Ok(quote::quote! {
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

            impl #impl_generics ::sov_modules_api::GenesisParamsTrait for GenesisConfig #type_generics #where_clause {
                fn genesis_slot_number(&self) -> u64 {
                    self.#chain_state_field_name.genesis_da_height as u64
                }
            }
        })
    }
}
