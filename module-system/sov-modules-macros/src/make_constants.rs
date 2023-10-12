use syn::{Ident, Type};

use crate::manifest::Manifest;

pub(crate) fn make_const(
    field_ident: &Ident,
    ty: &Type,
    vis: syn::Visibility,
    attrs: &[syn::Attribute],
) -> Result<proc_macro2::TokenStream, syn::Error> {
    Manifest::read_constants(field_ident)?.parse_constant(ty, field_ident, vis, attrs)
}
