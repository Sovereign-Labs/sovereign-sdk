use proc_macro2::Ident;
use quote::format_ident;
use syn::{DataStruct, GenericParam, Generics, ImplGenerics, TypeGenerics, WhereClause};
#[derive(Clone)]
pub(crate) struct StructNamedField {
    pub(crate) ident: proc_macro2::Ident,
    pub(crate) ty: syn::Type,
}

pub(crate) struct StructFieldExtractor {
    macro_name: &'static str,
}

impl StructFieldExtractor {
    pub(crate) fn new(macro_name: &'static str) -> Self {
        Self { macro_name }
    }

    // Extracts named fields form a struct or emits an error.
    pub(crate) fn get_fields_from_struct(
        &self,
        data: &syn::Data,
    ) -> Result<Vec<StructNamedField>, syn::Error> {
        match data {
            syn::Data::Struct(data_struct) => self.get_fields_from_data_struct(data_struct),
            syn::Data::Enum(en) => Err(syn::Error::new_spanned(
                en.enum_token,
                format!("The {} macro supports structs only.", self.macro_name),
            )),
            syn::Data::Union(un) => Err(syn::Error::new_spanned(
                un.union_token,
                format!("The {} macro supports structs only.", self.macro_name),
            )),
        }
    }

    fn get_fields_from_data_struct(
        &self,
        data_struct: &DataStruct,
    ) -> Result<Vec<StructNamedField>, syn::Error> {
        let mut output_fields = Vec::default();

        for original_field in data_struct.fields.iter() {
            let field_ident = original_field
                .ident
                .as_ref()
                .ok_or(syn::Error::new_spanned(
                    &original_field.ident,
                    format!(
                        "The {} macro supports structs only, unnamed fields witnessed.",
                        self.macro_name
                    ),
                ))?;

            let field = StructNamedField {
                ident: field_ident.clone(),
                ty: original_field.ty.clone(),
            };

            output_fields.push(field);
        }
        Ok(output_fields)
    }
}

pub(crate) const CALL: &str = "Call";
pub(crate) const QUERY: &str = "Query";

/// Represents "parsed" rust struct.
pub(crate) struct StructDef<'a> {
    pub(crate) ident: proc_macro2::Ident,
    pub(crate) impl_generics: ImplGenerics<'a>,
    pub(crate) type_generics: TypeGenerics<'a>,
    pub(crate) generic_param: &'a Ident,
    pub(crate) fields: Vec<StructNamedField>,
    pub(crate) where_clause: Option<&'a WhereClause>,
}

impl<'a> StructDef<'a> {
    pub(crate) fn new(
        ident: proc_macro2::Ident,
        fields: Vec<StructNamedField>,
        impl_generics: ImplGenerics<'a>,
        type_generics: TypeGenerics<'a>,
        generic_param: &'a Ident,
        where_clause: Option<&'a WhereClause>,
    ) -> Self {
        Self {
            ident,
            fields,
            impl_generics,
            type_generics,
            generic_param,
            where_clause,
        }
    }

    /// Creates an enum type based on the underlying struct.
    pub(crate) fn create_enum(
        &self,
        enum_legs: &[proc_macro2::TokenStream],
        postfix: &'static str,
    ) -> proc_macro2::TokenStream {
        let enum_ident = self.enum_ident(postfix);
        let impl_generics = &self.impl_generics;
        let where_clause = &self.where_clause;

        quote::quote! {
            // This is generated code (won't be exposed to the users) and we allow non camel case for enum variants.
            #[allow(non_camel_case_types)]
            // TODO we should not hardcode the serialization format inside the macro:
            // https://github.com/Sovereign-Labs/sovereign/issues/97
            #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, ::core::fmt::Debug, PartialEq)]
            enum #enum_ident #impl_generics #where_clause {
                #(#enum_legs)*
            }
        }
    }

    pub(crate) fn enum_ident(&self, postfix: &'static str) -> Ident {
        let ident = &self.ident;
        format_ident!("{ident}{postfix}")
    }
}

/// Gets type parameter from `Generics` declaration.
pub(crate) fn parse_generic_params(generics: &Generics) -> Result<Ident, syn::Error> {
    let generic_param = match generics.params.first().unwrap() {
        GenericParam::Type(ty) => &ty.ident,
        GenericParam::Lifetime(lf) => {
            return Err(syn::Error::new_spanned(
                lf,
                "Lifetime parameters not supported.",
            ))
        }
        GenericParam::Const(cnst) => {
            return Err(syn::Error::new_spanned(
                cnst,
                "Const parameters not supported.",
            ))
        }
    };

    Ok(generic_param.clone())
}
