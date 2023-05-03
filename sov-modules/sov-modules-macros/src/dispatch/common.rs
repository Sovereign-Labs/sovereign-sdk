use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, ToTokens};
use syn::{DataStruct, GenericParam, Generics, ImplGenerics, Meta, TypeGenerics, WhereClause};

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
        serialization_attrs: &Vec<TokenStream>,
    ) -> proc_macro2::TokenStream {
        let enum_ident = self.enum_ident(postfix);
        let impl_generics = &self.impl_generics;
        let where_clause = &self.where_clause;

        quote::quote! {
            // This is generated code (won't be exposed to the users) and we allow non camel case for enum variants.
            #[allow(non_camel_case_types)]
            #[derive(::core::fmt::Debug, PartialEq, #(#serialization_attrs),*)]
            pub enum #enum_ident #impl_generics #where_clause {
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

pub fn get_attribute_values(item: &syn::DeriveInput, attribute_name: &str) -> Vec<TokenStream> {
    let mut values = vec![];

    // Find the attribute with the given name on the root item
    if let Some(attr) = item
        .attrs
        .iter()
        .find(|attr| attr.path.is_ident(attribute_name))
    {
        if let Ok(meta) = attr.parse_meta() {
            if let Meta::List(list) = meta {
                values.extend(list.nested.iter().map(|n| {
                    let mut tokens = TokenStream::new();
                    n.to_tokens(&mut tokens);
                    return tokens;
                }));
            }
        }
    }

    values
}

pub fn get_serialization_attrs(item: &syn::DeriveInput) -> Result<Vec<TokenStream>, syn::Error> {
    const SERIALIZE: &str = "Serialize";
    const DESERIALIZE: &str = "Deserialize";

    let serialization_attrs = get_attribute_values(item, "serialization");

    let mut has_serialize = false;
    let mut has_deserialize = false;
    let mut has_other = false;

    let attributes: Vec<String> = serialization_attrs.iter().map(|t| t.to_string()).collect();

    for attr in &attributes {
        if attr.contains(SERIALIZE) {
            has_serialize = true;
        } else if attr.contains(DESERIALIZE) {
            has_deserialize = true;
        } else {
            has_other = true;
        }
    }

    let tokens: TokenStream = quote::quote! { serialization };
    if !has_serialize || !has_deserialize {
        return Err(syn::Error::new_spanned(
            &tokens,
            format!(
                "Serialization attributes must contain both '{}' and '{}', but contains '{:?}'",
                SERIALIZE, DESERIALIZE, &attributes
            ),
        ));
    } else if has_other {
        return Err(syn::Error::new_spanned(
            &tokens,
            format!("Serialization attributes can not contain attributes that are not '{}' and '{}', but contains: '{:?}'", 
                SERIALIZE, DESERIALIZE, &attributes.iter().filter(|a| !a.contains(SERIALIZE) && !a.contains(DESERIALIZE)).collect::<Vec<_>>()),
        ));
    }

    Ok(serialization_attrs)
}
