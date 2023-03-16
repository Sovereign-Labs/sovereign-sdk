use super::common::parse_generic_params;
use super::common::{StructFieldExtractor, StructNamedField};
use syn::DeriveInput;

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

        let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

        let fields = self.field_extractor.get_fields_from_struct(&data)?;
        let genesis_fn_body = Self::make_genesis_fn_body(&fields);
        let generic_param = parse_generic_params(&generics)?;

        // Implements the Genesis trait
        Ok(quote::quote! {
            impl #impl_generics sov_modules_api::Genesis for #ident #type_generics #where_clause {
                type Context = #generic_param;

                fn genesis(storage: <<Self as sov_modules_api::Genesis>::Context as sov_modules_api::Spec>::Storage) -> core::result::Result<(), sov_modules_api::Error> {
                    #(#genesis_fn_body)*
                    Ok(())
                }
            }
        }
        .into())
    }

    fn make_genesis_fn_body(fields: &[StructNamedField]) -> Vec<proc_macro2::TokenStream> {
        fields
            .iter()
            .map(|field| {
                let ident = &field.ident;
                let ty = &field.ty;

                // generates body for `genesis` method:
                //  let mut module_name = <ModuleName::<C> as sov_modules_api::ModuleInfo>::new(storage.clone());
                //  module_name.genesis()?;
                quote::quote! {
                    let mut #ident = <#ty as sov_modules_api::ModuleInfo>::new(storage.clone());
                    #ident.genesis()?;
                }
            })
            .collect()
    }
}
