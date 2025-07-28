use convert_case::{Case, Casing};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, ToTokens};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::token::Comma;
use syn::{DataStruct, GenericParam, ImplGenerics, Meta, TypeGenerics, Visibility};

#[derive(Clone)]
pub(crate) struct StructNamedField {
    pub(crate) ident: Ident,
    pub(crate) ty: syn::Type,
    pub(crate) attrs: Vec<syn::Attribute>,
    pub(crate) vis: Visibility,
}

impl StructNamedField {
    #[cfg_attr(not(feature = "native"), allow(unused))]
    pub(crate) fn contains_attr(&self, attr_ident: &str) -> bool {
        self.attrs
            .iter()
            .any(|attr| attr.path().is_ident(attr_ident))
    }
}

impl ToTokens for StructNamedField {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let docs = &self.attrs;
        let vis = &self.vis;
        let ident = &self.ident;
        let ty = &self.ty;
        tokens.extend(quote::quote! {
            #( #docs )*
            #vis #ident: #ty
        });
    }
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
    ) -> syn::Result<Vec<StructNamedField>> {
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
    ) -> syn::Result<Vec<StructNamedField>> {
        let mut output_fields = Vec::default();

        for original_field in &data_struct.fields {
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
                attrs: original_field.attrs.clone(),
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
    pub(crate) ident: Ident,
    pub(crate) impl_generics: ImplGenerics<'a>,
    pub(crate) type_generics: TypeGenerics<'a>,
    pub(crate) generic_param: &'a Ident,
    pub(crate) fields: Vec<StructNamedField>,
    pub(crate) where_clause: Option<&'a syn::WhereClause>,
}

impl<'a> StructDef<'a> {
    pub(crate) fn new(
        ident: Ident,
        fields: Vec<StructNamedField>,
        impl_generics: ImplGenerics<'a>,
        type_generics: TypeGenerics<'a>,
        generic_param: &'a Ident,
        where_clause: Option<&'a syn::WhereClause>,
    ) -> Self {
        Self {
            ident,
            impl_generics,
            type_generics,
            generic_param,
            fields,
            where_clause,
        }
    }

    /// Creates an enum type based on the underlying struct.
    pub(crate) fn create_enum(
        &self,
        enum_legs: &[TokenStream],
        enum_to_inner_legs: &[TokenStream],
        postfix: &'static str,
        extra_attributes: &[TokenStream],
    ) -> TokenStream {
        let enum_ident = self.enum_ident(postfix);
        let enum_discriminants_ident =
            &format_ident!("{}Discriminants", &enum_ident, span = Span::call_site());
        let impl_generics = &self.impl_generics;
        let where_clause = &self.where_clause;
        let type_generics = &self.type_generics;
        quote::quote! {
            #(#extra_attributes)*
            pub enum #enum_ident #impl_generics #where_clause {
                #(#enum_legs)*
            }

            impl #impl_generics ::sov_modules_api::NestedEnumUtils for #enum_ident #type_generics #where_clause {
                type Discriminants = #enum_discriminants_ident;

                fn discriminant(&self) -> Self::Discriminants {
                    self.into()
                }

                fn raw_contents(&self) -> &dyn ::std::any::Any {
                    match self {
                        #(#enum_to_inner_legs)*
                    }
                }
            }
        }
    }

    pub(crate) fn enum_ident(&self, postfix: &'static str) -> Ident {
        pascal_case_ident(&format_ident!(
            "{}{postfix}",
            &self.ident,
            span = Span::call_site()
        ))
    }
}

/// Returns the first match of the subattribute named `subattr_name` from the attribute `attr_name`.
/// There should be only one attribute with `attr_name`.
/// If no attribute with `attr_name` is found, returns `default_value`.
pub(crate) fn get_derived_struct_subattr<T: syn::parse::Parse>(
    input: &syn::DeriveInput,
    attr_name: &str,
    subattr_name: &str,
    default_value: T,
) -> syn::Result<T> {
    let attributes: Vec<_> = input
        .attrs
        .clone()
        .into_iter()
        .filter(|attr| attr.path().is_ident(attr_name))
        .collect();

    if attributes.len() > 1 {
        return Err(syn::Error::new_spanned(
            input.clone(),
            format!("Only one `#[{attr_name}]` attribute is allowed per type"),
        ));
    }

    if let Some(attr) = attributes.first() {
        let subattrs_parsed: Punctuated<syn::MetaNameValue, syn::Token![,]> = attr
            .parse_args_with(Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated)?;

        for subattr in subattrs_parsed {
            let syn::MetaNameValue { path, value, .. } = subattr;

            if path.is_ident(subattr_name) {
                return Ok(syn::parse_quote! { #value });
            }
        }
    }

    Ok(default_value)
}

/// Gets the first type parameter's identifier from [`syn::Generics`].
pub(crate) fn get_generics_type_param(
    generics: &syn::Generics,
    error_span: Span,
) -> syn::Result<Ident> {
    let generic_param = match generics
        .params
        .first()
        .ok_or_else(|| syn::Error::new(error_span, "No generic parameters found"))?
    {
        GenericParam::Type(ty) => &ty.ident,
        GenericParam::Lifetime(lf) => {
            return Err(syn::Error::new_spanned(
                lf,
                "Lifetime parameters are not supported.",
            ))
        }
        GenericParam::Const(cnst) => {
            return Err(syn::Error::new_spanned(
                cnst,
                "Const parameters are not supported.",
            ))
        }
    };

    Ok(generic_param.clone())
}

pub(crate) fn get_derived_enum_attrs(
    ident: &str,
    input: &syn::DeriveInput,
    mut default_attrs: Vec<TokenStream>,
) -> syn::Result<Vec<TokenStream>> {
    let mut attributes = Vec::new();
    let mut found_opt_out = false;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident(ident))
    {
        let event_attrs = attr.parse_args_with(Punctuated::<Meta, Comma>::parse_terminated)?;
        for event_attr in event_attrs {
            match event_attr {
                Meta::Path(path) => {
                    if path.is_ident("no_default_attrs") {
                        found_opt_out = true;
                    } else {
                        attributes.push(quote::quote! {#[#path]});
                    }
                }
                Meta::List(list) => attributes.push(quote::quote! {#[#list]}),
                Meta::NameValue(value) => attributes.push(quote::quote! {#[#value]}),
            }
        }
    }
    if found_opt_out {
        Ok(attributes)
    } else {
        // Put the default attributes first to avoid "warning: derive helper attribute is used before it is introduced"
        // if the user specifies a `serde` or `borsh` attribute
        default_attrs.extend(attributes);
        Ok(default_attrs)
    }
}

// Converts a Rust identifier into a human-readable URL path segment on a
// "best-effort" basis (i.e. some identifiers may result in invalid or unreadable
// URLs).
pub fn str_to_url_segment(ident: &Ident) -> String {
    ident.to_string().replace('_', "-")
}

/// Converts an identifier to `PascalCase`.
pub fn pascal_case_ident(ident: &Ident) -> Ident {
    Ident::new(&ident.to_string().to_case(Case::Pascal), ident.span())
}

/// Wraps the code in a new scope using the `const _: () = {};` trick.
///
/// Adapted from MIT-licensed code here:
/// <https://github.com/serde-rs/serde/blob/3202a6858a2802b5aba2fa5cf3ec8f203408db74/serde_derive/src/dummy.rs#L15-L22>.
///
/// Copyright (c) David Tolnay and Serde contributors.
pub fn wrap_in_new_scope(code: &TokenStream) -> TokenStream {
    quote::quote! {
        #[doc(hidden)]
        #[allow(all, clippy::all)] // <-- just to make sure rustc doesn't complain about generated code.
        const _: () = {
            #code
        };
    }
}

/// Returns a list of all `#[doc = "..."`] attributes found in the original list
/// of attributes.
#[cfg(feature = "native")]
pub fn doc_attributes(attrs: &[syn::Attribute]) -> Vec<syn::Attribute> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .cloned()
        .collect::<Vec<_>>()
}

/// Iterates over all `#[doc = "..."`] attributes and concatenates their inner
/// string values, separated by newlines.
pub fn join_doc_comments(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    // Collect string literals from doc comments
    let string_literals = attrs
        .iter()
        .filter_map(|attr| {
            // Match attributes that are doc comments
            if let Meta::NameValue(name_value) = &attr.meta {
                if name_value.path.is_ident("doc") {
                    Some(name_value.value.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .map(|expr| {
            // Extract the string literal from the expression
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit_str),
                ..
            }) = expr
            {
                Ok(lit_str.value())
            } else {
                Err(syn::Error::new(
                    expr.span(),
                    "Doc comment is not a string literal",
                ))
            }
        })
        .collect::<syn::Result<Vec<_>>>()?;

    if string_literals.is_empty() {
        return Ok(None);
    }

    // Process the collected strings
    let trimmed: Vec<_> = string_literals
        .iter()
        .flat_map(|s| s.split('\n'))
        .map(|line| line.trim().to_string())
        .collect();

    Ok(Some(trimmed.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_generic_type_param_success() {
        // tests for get_generics_type_param
        let generics = syn::parse_quote! {
            <T: Trait>
        };

        let generic_param = get_generics_type_param(&generics, Span::call_site()).unwrap();
        assert_eq!(generic_param, "T");
    }

    #[test]
    fn get_generic_type_param_first_lifetime() {
        let generics = syn::parse_quote! {
            <'a, T: Trait>
        };
        let generic_param = get_generics_type_param(&generics, Span::call_site());
        let error = generic_param.unwrap_err();
        assert_eq!(error.to_string(), "Lifetime parameters are not supported.");
    }

    #[test]
    fn get_generic_type_param_first_const() {
        // error test case for get_generics_type_param when the first generic param is const
        let generics = syn::parse_quote! {
            <const T: Trait>
        };
        let generic_param = get_generics_type_param(&generics, Span::call_site());

        let error = generic_param.unwrap_err();
        assert_eq!(error.to_string(), "Const parameters are not supported.");
    }
}
