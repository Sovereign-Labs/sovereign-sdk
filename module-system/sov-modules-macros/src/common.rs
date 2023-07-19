use std::collections::HashMap;

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, ToTokens};
use syn::punctuated::Punctuated;
use syn::{
    DataStruct, GenericParam, Generics, ImplGenerics, Meta, TypeGenerics, TypeParamBound,
    WhereClause, WherePredicate,
};

#[derive(Clone)]
pub(crate) struct StructNamedField {
    pub(crate) ident: proc_macro2::Ident,
    pub(crate) ty: syn::Type,
    pub(crate) vis: syn::Visibility,
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
                vis: original_field.vis.clone(),
            };

            output_fields.push(field);
        }
        Ok(output_fields)
    }
}

pub(crate) const CALL: &str = "Call";

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

/// Gets the type parameter's identifier from [`syn::Generics`].
pub(crate) fn get_generics_type_param(
    generics: &Generics,
    error_span: Span,
) -> Result<Ident, syn::Error> {
    let generic_param = match generics
        .params
        .first()
        .ok_or_else(|| syn::Error::new(error_span, "No generic parameters found"))?
    {
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
        if let Ok(Meta::List(list)) = attr.parse_meta() {
            values.extend(list.nested.iter().map(|n| {
                let mut tokens = TokenStream::new();
                n.to_tokens(&mut tokens);
                tokens
            }));
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

// pub struct GenericWithBounds {
//     pub ident: Ident,
//     pub bounds: Vec<TypeParamBound>,
// }

// pub struct GenericTypesWithBounds {
//     /// A mapping from a generic type's ident to a complete list of its bounds
//     pub bounds: HashMap<Ident, Punctuated<TypeParamBound, syn::token::Add>>,
// }

/// Extract a mapping from generic types to their associated trait bounds, including
/// the ones from the where clause.
///
/// For example, given the following struct:
/// ```rust,ignore
/// use sov_modules_macros::common::GenericTypesWithBounds;
/// let test_struct: syn::ItemStruct = syn::parse_quote! {
///     struct TestStruct<T: SomeTrait> where T: SomeOtherTrait {
///         field: T
///     }
/// };
/// // We want to extract both the inline bounds, and the bounds from the where clause...
/// // so that the generics from above definition are equivalent what we would have gotten
/// // from writing `T: SomeTrait + SomeOtherTrait` inline
/// let desired_bounds_for_t: syn::TypeParam = syn::parse_quote!(T: SomeTrait + SomeThirdTrait);
///
/// // That is exactly what `GenericTypesWithBounds` does
/// let our_bounds = extract_generic_type_bounds(&test_struct.generics);
/// assert_eq!(our_bounds.get(T), Some(&desired_bounds_for_t.bounds));
/// ```
pub fn extract_generic_type_bounds(
    generics: &Generics,
) -> HashMap<Ident, Punctuated<TypeParamBound, syn::token::Add>> {
    let mut generics_with_bounds: HashMap<_, _> = Default::default();
    // Collect the inline bounds from each generic param
    for param in generics.params.iter() {
        match param {
            GenericParam::Type(ty) => {
                generics_with_bounds.insert(ty.ident.clone(), ty.bounds.clone());
            }
            _ => {}
        }
    }

    // Iterate over the bounds in the `where_clause` and add them to the map
    if let Some(where_clause) = &generics.where_clause {
        for predicate in &where_clause.predicates {
            match &predicate {
                WherePredicate::Type(predicate_type) => {
                    // If the bounded type is a regular type path, we need to extract the bounds and add them to the map.
                    // For now, we ignore more exotic bounds `[T; N]: SomeTrait`.
                    if let syn::Type::Path(type_path) = &predicate_type.bounded_ty {
                        // Add the bounds from this type into the map
                        let ident = extract_ident(type_path);
                        if let Some(bounds) = generics_with_bounds.get_mut(ident) {
                            bounds.extend(predicate_type.bounds.iter().cloned())
                        }
                    }
                }
                // We can ignore lifetimes and "Eq" predicates since they don't add any trait bounds
                _ => {}
            }
        }
    }
    generics_with_bounds
}

/// Extract the type ident from a `TypePath`.
pub fn extract_ident(type_path: &syn::TypePath) -> &Ident {
    &type_path
        .path
        .segments
        .last()
        .expect("Type path must have at least one segment")
        .ident
}

#[test]
fn test_generic_types_with_bounds() {
    let test_struct: syn::ItemStruct = syn::parse_quote! {
        struct TestStruct<T: SomeTrait, U: SomeOtherTrait, V> where T: SomeThirdTrait {
            field: (T, U, V)
        }
    };
    let generics = test_struct.generics;
    let our_bounds = extract_generic_type_bounds(&generics);
    let expected_bounds_for_t: syn::TypeParam = syn::parse_quote!(T: SomeTrait + SomeThirdTrait);
    let expected_bounds_for_u: syn::TypeParam = syn::parse_quote!(U: SomeOtherTrait);

    assert_eq!(
        our_bounds.get(&format_ident!("T")),
        Some(&expected_bounds_for_t.bounds)
    );
    assert_eq!(
        our_bounds.get(&format_ident!("U")),
        Some(&expected_bounds_for_u.bounds)
    );
    assert_eq!(
        our_bounds.get(&format_ident!("V")),
        Some(&syn::punctuated::Punctuated::new())
    );
}
