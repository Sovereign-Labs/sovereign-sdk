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
///  * FromField(index)`: parses the field at location `index` in the parent struct as a single
///  `u8` byte and uses that as the amount of decimal places. Performs NO type-checking.
#[derive(Debug, Clone)]
pub enum FixedPointDisplay {
    Direct(u8),
    FromField {
        field_index: SpannedValue<usize>,
        byte_offset: usize,
    },
}

impl FixedPointDisplay {
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
                        let field_meta = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;
                        let (index, offset) = match field_meta.len() {
                            1 => {
                                (<usize as FromMeta>::from_expr(&field_meta[0])?, 0)
                            },
                            2 => {
                                let index = <usize as FromMeta>::from_expr(&field_meta[0])?;
                                match &field_meta[1] {
                                    Expr::Assign(ExprAssign {
                                        left,
                                        right,
                                        ..
                                    }) if matches!(
                                        **left,
                                        Expr::Path(ExprPath { ref path, .. }
                                    ) if path
                                        .get_ident()
                                        .map(|ident| ident.to_string())
                                        .is_some_and(|s| s == "offset")
                                    ) => {
                                        let offset = <usize as FromMeta>::from_expr(right)?;
                                        (index, offset)
                                    }
                                    _ => {
                                        return Err(darling::Error::unsupported_shape("Field references for fixed points must provide a field index: `from_field(1)`, and optionally a 0-indexed byte offset: `from_field(5, offset=31)`"));
                                    }
                                }
                            }
                            _ => {
                                return Err(darling::Error::unsupported_shape("Field references for fixed points must provide a field index: `from_field(1)`, and optionally a 0-indexed byte offset: `from_field(5, offset=31)`"));
                            }
                        };
                        Ok(FixedPointDisplay::FromField { field_index: SpannedValue::new(index, list.span()), byte_offset: offset })
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
