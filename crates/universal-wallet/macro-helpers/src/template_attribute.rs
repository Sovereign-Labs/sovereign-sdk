use std::collections::HashMap;

use darling::util::SpannedValue;
use darling::{Error, FromMeta, Result};
use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::{Expr, ExprCall, ExprLit, Ident, Lit, Meta, Token};

/// Attributes of the format
/// `#[sov_wallet(template("transfer" = value("hi"), "other_template" = input("msg")))]`
/// providing the annotations necessary to generate UniversalWallet::get_child_templates() for
/// fields which are part of standard templated transactions
#[derive(Debug, Default, Clone)]
pub struct TransactionTemplates {
    pub original: TokenStream,
    pub template: HashMap<String, InputOrValue>,
}

#[derive(Debug, Clone)]
pub enum InputOrValue {
    FieldNameInput,
    Input(String),
    Value(String),
    DefaultValue,
    BytesValue(SpannedValue<String>),
}

impl FromMeta for TransactionTemplates {
    /// This is parsed as a Punctuated of individual templates, each consisting of an Expr::Assign
    /// with the template name on the LHS and an Expr::Call for the InputOrValue on the RHS
    ///
    /// We need to override `from_meta` directly instead of using e.g. `from_list` because
    /// darling's parsing seems pretty pretty rigid. For example, `from_list` tries to parse it as
    /// `Punctuated::<NestedMeta, Token![,]>` which is too greedy, fails to parse the complex Expr
    /// and complains that it expects ',' at the first '='.
    fn from_meta(item: &Meta) -> Result<Self> {
        // Split it into a list of templates; each expr is an individual template annotation
        let list = match item {
            Meta::List(list) => Ok(list),
            Meta::Path(_) => Err(Error::unsupported_format("Path").with_span(item)),
            Meta::NameValue(_) => Err(Error::unsupported_format("NameValue").with_span(item)),
        }?;
        let exprs = syn::punctuated::Punctuated::<Expr, Token![,]>::parse_terminated
            .parse2(list.tokens.clone())?;

        let mut ret = HashMap::new();

        for expr in exprs {
            match expr {
                Expr::Assign(ref expr) => {
                    // LHS of assignment is template name
                    let template_name = parse_lit_str((*expr.left).clone())?;
                    if ret.contains_key(&template_name) {
                        return Err(Error::duplicate_field(&template_name).with_span(&expr));
                    }

                    // RHS is either:
                    //  - `input` to reuse the field name as name, or
                    //  - `input("name binding")`, or
                    //  - `value("value")`
                    let (discriminant, maybe_func) = match *expr.right {
                        // This unboxing is ugly...
                        Expr::Call(ref f @ ExprCall { ref func, .. }) => match **func {
                            Expr::Path(ref p) => {
                                p.path.get_ident().cloned().map(|i| (i, Some(f.clone())))
                            }
                            _ => None,
                        },
                        Expr::Path(ref p) => p.path.get_ident().cloned().map(|i| (i, None)),
                        _ => None,
                    }
                    .ok_or(Error::unexpected_expr_type(&expr.right).with_span(&expr.right))?;

                    if let Some(func) = maybe_func {
                        // Template binding is call-type: `input("arg")` or `value("arg")`

                        // And we need exactly one string value between the paranthesis
                        ensure_exprcall_has_one_arg(&func)?;

                        // TODO: support for Assign values here for `json = ""` and `hex = ""` pre-encoded values
                        let arg = func.args[0].clone();
                        match discriminant.to_string().as_str() {
                            "input" => {
                                let input = parse_lit_str(arg)?;
                                ret.insert(template_name, InputOrValue::Input(input))
                            }
                            "value" => ret.insert(template_name, parse_value(arg)?),
                            s => return Err(Error::unknown_field(s).with_span(&discriminant)),
                        };
                    } else {
                        // Template binding is single value
                        // Only valid value is `input`
                        match discriminant.to_string().as_str() {
                            "input" => ret.insert(template_name, InputOrValue::FieldNameInput),
                            "value" => {
                                return Err(Error::custom("`value` bindings must specify a value")
                                    .with_span(&discriminant))
                            }
                            s => return Err(Error::unknown_field(s).with_span(&discriminant)),
                        };
                    }

                    Ok(())
                }
                _ => Err(Error::unexpected_expr_type(&expr).with_span(&expr)),
            }?;
        }
        Ok(TransactionTemplates {
            template: ret,
            original: item.to_token_stream(),
        })
    }
}

fn ensure_exprcall_has_one_arg(e: &ExprCall) -> Result<()> {
    // For some reason, ExprCall.args doesn't have the right span
    // information, so we set the errors to the ExprCall itself
    match e.args.len().cmp(&1) {
        std::cmp::Ordering::Less => Err(Error::too_few_items(1).with_span(&e)),
        std::cmp::Ordering::Greater => Err(Error::too_many_items(1).with_span(&e)),
        _ => Ok(()),
    }
}

fn parse_lit_str(e: Expr) -> Result<String> {
    match e {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.value()),
        Expr::Lit(expr) => Err(Error::unexpected_lit_type(&expr.lit)),
        _ => Err(Error::unexpected_expr_type(&e).with_span(&e)),
    }
}

// Helper to parse the possible Value options of an InputOrValue
fn parse_value(e: Expr) -> Result<InputOrValue> {
    match e {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(InputOrValue::Value(s.value())),
        Expr::Lit(expr) => Err(Error::unexpected_lit_type(&expr.lit)),
        Expr::Path(p) if p.path.is_ident("default") => Ok(InputOrValue::DefaultValue),
        Expr::Path(p) => {
            Err(Error::unknown_value(p.path.to_token_stream().to_string().as_str()).with_span(&p))
        }
        Expr::Call(e) => match get_exprcall_path(&e)?.to_string().as_str() {
            "bytes" => {
                ensure_exprcall_has_one_arg(&e)?;
                let bytes = parse_lit_str(e.args[0].clone())?;
                Ok(InputOrValue::BytesValue(SpannedValue::new(
                    bytes,
                    e.args[0].span(),
                )))
            }
            s => Err(Error::unknown_value(s).with_span(&e)),
        },
        _ => Err(Error::unexpected_expr_type(&e).with_span(&e)),
    }
}

// Helper to get the "function" name in a `func(arg, arg...)` expression
fn get_exprcall_path(e: &ExprCall) -> Result<Ident> {
    match *e.func {
        Expr::Path(ref p) => p.path.get_ident().cloned(),
        _ => None,
    }
    .ok_or(Error::unexpected_expr_type(&e.func).with_span(&e.func))
}
