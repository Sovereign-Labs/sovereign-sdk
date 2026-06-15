use darling::ast::NestedMeta;
use darling::util::SpannedValue;
use darling::{Error, FromMeta};
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Expr, ExprAssign, ExprPath, Ident, Meta, Token};

/// A specification for the decimal points to use for display of an integer.
///  * `Direct(d)`: uses `d` decimal places
///  * `FromField(index)`: parses the field at location `index` in the parent struct as a single
///    `u8` byte and uses that as the amount of decimal places. Performs NO type-checking.
///  * `FromFieldWithOverride`: like `FromField`, but if the entire sibling field equals the
///    `override_match` expression, uses `override_decimals` instead of the byte.
#[derive(Debug, Clone)]
pub enum FixedPointDisplay {
    Direct(u8),
    FromField {
        field_index: SpannedValue<usize>,
        byte_offset: usize,
    },
    FromFieldWithOverride {
        field_index: SpannedValue<usize>,
        byte_offset: usize,
        override_match: Box<Expr>,
        override_decimals: Box<Expr>,
    },
}

impl FixedPointDisplay {
    /// The sibling field this display reads bytes from, if any. Used to mark the parent struct as
    /// `peekable` and to bounds-check the field index.
    pub fn field_index(&self) -> Option<&SpannedValue<usize>> {
        match self {
            FixedPointDisplay::Direct(_) => None,
            FixedPointDisplay::FromField { field_index, .. }
            | FixedPointDisplay::FromFieldWithOverride { field_index, .. } => Some(field_index),
        }
    }

    pub fn resolve(&self, crate_prefix: &Option<syn::TypePath>) -> TokenStream {
        match self {
            FixedPointDisplay::Direct(decimals) => {
                quote! { #crate_prefix::sov_universal_wallet::ty::IntegerDisplay::FixedPoint(
                    #crate_prefix::sov_universal_wallet::ty::FixedPointDisplay::Decimals(#decimals)
                )}
            }
            FixedPointDisplay::FromField {
                field_index,
                byte_offset,
            } => {
                let field_index = **field_index;
                quote! { #crate_prefix::sov_universal_wallet::ty::IntegerDisplay::FixedPoint(
                    #crate_prefix::sov_universal_wallet::ty::FixedPointDisplay::FromSiblingField {
                        field_index: #field_index,
                        byte_offset: #byte_offset
                    }
                )}
            }
            FixedPointDisplay::FromFieldWithOverride {
                field_index,
                byte_offset,
                override_match,
                override_decimals,
            } => {
                let field_index = **field_index;
                quote! { #crate_prefix::sov_universal_wallet::ty::IntegerDisplay::FixedPoint(
                    #crate_prefix::sov_universal_wallet::ty::FixedPointDisplay::FromSiblingFieldWithOverride {
                        field_index: #field_index,
                        byte_offset: #byte_offset,
                        override_match: #override_match,
                        override_decimals: #override_decimals
                    }
                )}
            }
        }
    }
}

impl FromMeta for FixedPointDisplay {
    fn from_list(items: &[NestedMeta]) -> darling::Result<Self> {
        ensure_list_has_one_arg(items)?;
        match &items[0] {
            NestedMeta::Lit(lit) => {
                let decimals = <u8 as FromMeta>::from_value(lit)?;
                if decimals > 39 {
                    return Err(darling::Error::custom("Invalid value: too many decimals. The maximum amount of decimals for the widest supported integer, u128, is 39.").with_span(lit));
                }
                Ok(FixedPointDisplay::Direct(decimals))
            }
            NestedMeta::Meta(Meta::List(list)) => {
                match list.path.get_ident().map(Ident::to_string) {
                    Some(s) if s == "from_field" => {
                        const USAGE: &str = "Field references for fixed points must provide a field index: `from_field(1)`, optionally a 0-indexed byte offset: `from_field(5, offset=31)`, and optionally an override pair: `from_field(1, offset=31, override_eq=<expr>, override_decimals=<expr>)`";
                        let field_meta = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;
                        if field_meta.is_empty() {
                            return Err(darling::Error::unsupported_shape(USAGE));
                        }
                        let index = <usize as FromMeta>::from_expr(&field_meta[0])?;
                        let mut offset = 0usize;
                        let mut override_match: Option<Expr> = None;
                        let mut override_decimals: Option<Expr> = None;
                        // Remaining args are `name = value` assignments.
                        for arg in field_meta.iter().skip(1) {
                            let Expr::Assign(ExprAssign { left, right, .. }) = arg else {
                                return Err(darling::Error::unsupported_shape(USAGE));
                            };
                            let Expr::Path(ExprPath { path, .. }) = left.as_ref() else {
                                return Err(darling::Error::unsupported_shape(USAGE));
                            };
                            match path.get_ident().map(Ident::to_string).as_deref() {
                                Some("offset") => offset = <usize as FromMeta>::from_expr(right)?,
                                Some("override_eq") => override_match = Some(right.as_ref().clone()),
                                Some("override_decimals") => override_decimals = Some(right.as_ref().clone()),
                                _ => return Err(darling::Error::unknown_field_path(path).with_span(&list)),
                            }
                        }
                        let field_index = SpannedValue::new(index, list.span());
                        match (override_match, override_decimals) {
                            (None, None) => Ok(FixedPointDisplay::FromField { field_index, byte_offset: offset }),
                            (Some(override_match), Some(override_decimals)) => Ok(FixedPointDisplay::FromFieldWithOverride {
                                field_index,
                                byte_offset: offset,
                                override_match: Box::new(override_match),
                                override_decimals: Box::new(override_decimals),
                            }),
                            _ => Err(darling::Error::custom("`override_eq` and `override_decimals` must be provided together").with_span(&list)),
                        }
                    }
                    Some(s) => Err(Error::unknown_value(&s).with_span(&list)),
                    None => Err(Error::unknown_field_path(&list.path))
                }
            }
            NestedMeta::Meta(ref m) => Err(Error::unsupported_shape("Fixed point display must either be a literal: `18`, or a field reference by index: `from_field(0)`").with_span(m))
        }
    }
}

pub(crate) fn ensure_list_has_one_arg(items: &[NestedMeta]) -> darling::Result<()> {
    match items.len().cmp(&1) {
        std::cmp::Ordering::Less => Err(Error::too_few_items(1)),
        std::cmp::Ordering::Greater => Err(Error::too_many_items(1)),
        std::cmp::Ordering::Equal => Ok(()),
    }
}
