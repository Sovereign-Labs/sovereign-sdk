use syn::parse::Parse;
use syn::{Attribute, Ident, Token, Type, Visibility};

use crate::manifest::Manifest;

/// A partial const declaration: `const MAX: u16;`.
pub struct PartialItemConst {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub const_token: Token![const],
    pub ident: Ident,
    pub colon_token: Token![:],
    pub ty: Type,
    pub semi_token: Token![;],
}

impl Parse for PartialItemConst {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            attrs: Attribute::parse_outer(input)?,
            vis: input.parse()?,
            const_token: input.parse()?,
            ident: input.parse()?,
            colon_token: input.parse()?,
            ty: input.parse()?,
            semi_token: input.parse()?,
        })
    }
}

pub(crate) fn make_const(
    field_ident: &Ident,
    ty: &Type,
    vis: syn::Visibility,
    attrs: &[syn::Attribute],
) -> Result<proc_macro2::TokenStream, syn::Error> {
    Manifest::read_constants(field_ident)?.parse_constant(ty, field_ident, vis, attrs)
}
