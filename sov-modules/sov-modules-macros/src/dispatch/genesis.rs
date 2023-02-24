use syn::DeriveInput;
use syn::TypeGenerics;

use super::common::{StructFieldExtractor, StructNamedField};

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
    ) -> Result<proc_macro::TokenStream, syn::Error> {
        let DeriveInput {
            data,
            ident,
            generics,
            ..
        } = input;

        let (impl_generics, type_generics, _) = generics.split_for_impl();

        let fields = self.field_extractor.get_fields_from_struct(&data)?;
        let genesis_fn_body = Self::make_genesis_fn_body(type_generics.clone(), &fields);

        // Implements the Genesis trait
        Ok(quote::quote! {
            impl #impl_generics sov_modules_api::Genesis for #ident #type_generics {

                type Context = C;
                type Config = <C::Storage as sov_state::Storage>::Config;

                fn genesis(config: Self::Config) -> core::result::Result<C::Storage, sov_modules_api::Error> {
                    let storage = <C::Storage as sov_state::Storage> ::new(config);
                    #(#genesis_fn_body)*
                    Ok(storage)
                }
            }
        }
        .into())
    }

    fn make_genesis_fn_body(
        type_generics: TypeGenerics,
        fields: &[StructNamedField],
    ) -> Vec<proc_macro2::TokenStream> {
        fields
            .iter()
            .map(|field| {
                let ident = &field.ident;
                let ty = &field.ty;

                // generates body for `genesis` method:
                //  let mut module_name = <ModuleName::<C> as sov_modules_api::ModuleInfo<C>>::new(storage.clone());
                //  module_name.genesis()?;
                 quote::quote! {
                    let mut #ident = <#ty as sov_modules_api::ModuleInfo #type_generics>::new(storage.clone());
                    #ident.genesis()?;
                }
            })
            .collect()
    }
}
