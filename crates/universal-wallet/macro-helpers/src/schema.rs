use std::ops::Deref;

use darling::ast::{self, Data, Fields, Style};
use darling::usage::{CollectLifetimes, CollectTypeParams, GenericsExt, Purpose};
use darling::util::SpannedValue;
use darling::{uses_lifetimes, uses_type_params, FromDeriveInput, FromField, FromVariant};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::token::{Comma, Where};
use syn::{
    DeriveInput, Expr, GenericParam, Generics, Ident, LitStr, Type, TypeArray, TypeGroup,
    TypeParen, TypePtr, TypeReference, TypeSlice, WhereClause, WherePredicate,
};

type SynResult<T> = Result<T, syn::Error>;

use darling::FromMeta;

use super::foreign_attributes::{parse_foreign_attrs, Serde};
use crate::fixed_point_ints::FixedPointDisplay;
use crate::foreign_attributes::ForeignAttrs;
use crate::template_attribute::{InputOrValue, TransactionTemplates};

#[derive(Debug, FromMeta, Default, Clone)]
#[darling(rename_all = "snake_case")]
pub enum ByteDisplayType {
    #[default]
    Hex,
    Decimal,
    Bech32 {
        #[darling(with = darling::util::parse_expr::parse_str_literal)]
        prefix: syn::Expr,
    },
    Bech32m {
        #[darling(with = darling::util::parse_expr::parse_str_literal)]
        prefix: syn::Expr,
    },
    Base58,
}

impl ByteDisplayType {
    pub fn resolve(&self, crate_prefix: &Option<syn::TypePath>) -> TokenStream {
        match self {
            ByteDisplayType::Hex => {
                quote! { #crate_prefix::sov_universal_wallet::ty::ByteDisplay::Hex }
            }
            ByteDisplayType::Decimal => {
                quote! { #crate_prefix::sov_universal_wallet::ty::ByteDisplay::Decimal }
            }
            ByteDisplayType::Bech32 { prefix } => {
                quote! { #crate_prefix::sov_universal_wallet::ty::ByteDisplay::Bech32 { prefix: #crate_prefix::sov_universal_wallet::bech32::Hrp::parse(#prefix).expect("Invalid bech32 prefix") } }
            }
            ByteDisplayType::Bech32m { prefix } => {
                quote! { #crate_prefix::sov_universal_wallet::ty::ByteDisplay::Bech32m { prefix: #crate_prefix::sov_universal_wallet::bech32::Hrp::parse(#prefix).expect("Invalid bech32 prefix") } }
            }
            ByteDisplayType::Base58 => {
                quote! { #crate_prefix::sov_universal_wallet::ty::ByteDisplay::Base58 }
            }
        }
    }

    pub fn len(&self, input: &SpannedValue<String>) -> Result<usize, darling::Error> {
        match self {
            ByteDisplayType::Hex => Ok(if input.starts_with("0x") {
                (input.len() - 2) / 2
            } else {
                input.len() / 2
            }),
            ByteDisplayType::Decimal => Ok(input.split(',').count()),
            ByteDisplayType::Bech32 { .. } | ByteDisplayType::Bech32m { .. } => {
                let (_, bytes) = bech32::decode(input).map_err(|e| {
                    darling::Error::custom(format!("Invalid bech32(m) literal value: {e}"))
                        .with_span(&input.span())
                })?;
                Ok(bytes.len())
            }
            ByteDisplayType::Base58 => Ok(bs58::decode(input.deref())
                .into_vec()
                .map_err(|e| {
                    darling::Error::custom(format!("Invalid base58 literal value: {e}"))
                        .with_span(&input.span())
                })?
                .len()),
        }
    }
}

#[derive(Debug, FromMeta, Default, Clone)]
pub struct Bounds(syn::punctuated::Punctuated<syn::WherePredicate, Comma>);

impl ToTokens for Bounds {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0.to_tokens(tokens)
    }
}

/// Derive the `UniversalWallet` trait on an input type. See the actual proc-macro definition for
/// usage information, including attributes; the code generation is documented here.
///
/// The macro generates the `UniversalWallet::scaffold()` and `UniversalWallet::get_child_links()`
/// methods on the trait. The scaffold is the Container corresponding to the data structure of
/// the derived-on type, with the types of all child fields (if any) set as placeholder links.
/// `get_child_links()` is what allows schema generation to recursively walk every child type and
/// replace the placeholder links with a link to its corresponding schema.
///
/// For struct types, the generation is relatively self-explanatory: an `impl UniversalWallet` is
/// emitted, containing the above two functions. However, enums are treated differently: any
/// variants that themselves contain fields are treated as containing a virtual struct. A struct
/// definition is then emitted, containing the actual data for that variant, with the proc macro
/// being in turn derived on it; the emitted code thus contains an impl for the enum type, structs
/// for every variant that contains fields, and corresponding impls for each of those structs.
pub fn derive(
    input: DeriveInput,
    prefix: Option<syn::TypePath>,
    macro_name: TokenStream,
) -> Result<TokenStream, syn::Error> {
    let res = derive_wallet_field(input, prefix, macro_name)?;
    if std::env::var("SOV_EXPAND_PROC_MACROS").is_ok() {
        println!("------ Generated Wallet Output -------\n\n{}\n\n------ End Generated Wallet Output -------",&res);
    }
    Ok(res)
}

/// Collect the tokens to implement the return value of UniversalWallet::get_child_links for a
/// struct type
fn struct_child_links(
    input: &Fields<InputField>,
    prefix: &Option<syn::TypePath>,
) -> Vec<TokenStream> {
    input
        .fields
        .iter()
        .filter(|field| !field.skip)
        .map(|item| maybe_generate_field_schema(item, prefix))
        .collect::<Vec<_>>()
}

/// Generates the bulk of the body of `get_child_templates` for a struct type.
/// Based on the attribute annotations, fields get a direct template constructed for them, either
/// an input binding or a pre-encoded default value. (In the latter case, this function inserts
/// code to encode the value based on the string provided in the attribute.) A vector of every
/// per-field template chunk so defined by the attributes is formed, and inserted alongside a call
/// to get_child_templates into the `AttributeAndChildTemplateSet` data structure, which can then
/// be used in further logic to merge the chunks and construct a full template.
/// A call to `fill_links` is also inserted so that the Link::Placeholder values that get
/// constructed in the chunk can be filled with proper types from the schema, at schema (and
/// template) generation time (i.e. at runtime).
fn struct_child_templates(
    input: &Fields<InputField>,
    schema_arg: &Ident,
    input_name_str: &String,
    prefix: &Option<syn::TypePath>,
) -> Result<TokenStream, darling::Error> {
    let child_templates = input.fields.iter().enumerate().filter(|(_, field)| !field.skip).map(|(i, field)| {
        let ty = match &field.template_override_ty {
            Some(ty) => ty.to_token_stream(),
            None => field.ty_tokens(),
        };
        // For every template annotation (will do nothing if there's no attribute)...
        let templates: Vec<TokenStream> = field.template.template.iter().map(|t| {
            let field_name = field.ident.clone().ok_or(darling::Error::custom("`input` bindings with no explicit name can only be used for named fields").with_span(&field.ident))?.to_string();
            // build code to construct the template chunk based on the data from the attribute
            let transaction_template = match t.1 {
                InputOrValue::FieldNameInput => quote! {
                    #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplate::from_input(#field_name.to_string(), #i)
                },
                InputOrValue::Input(input) => quote! {
                    #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplate::from_input(#input.to_string(), #i)
                },
                InputOrValue::Value(value) => quote! {
                    #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplate::from_bytes(
                        ::borsh::to_vec(
                            &<#ty as ::core::str::FromStr>::from_str(#value).expect(format!("String parsing failed for value: {} while constructing schema template on type {}", #value, #input_name_str).as_str())
                        ).expect(format!("Borsh-encoding of value {} failed while encoding schema template on type {}", #value, #input_name_str).as_str())
                    )
                },
                InputOrValue::DefaultValue => quote! {
                    #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplate::from_bytes(
                        ::borsh::to_vec(
                            &<#ty as ::core::default::Default>::default()
                        ).expect(format!("Borsh-encoding of default value failed while encoding schema template on type {}", #input_name_str).as_str())
                    )
                },
                InputOrValue::BytesValue(bytes) => {
                    let display_type = field.display.clone().unwrap_or(ByteDisplayType::Hex);
                    let byte_display = display_type.resolve(prefix);
                    let const_len = display_type.len(bytes)?;
                    let bytes_str = bytes.deref();
                    quote! {
                        #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplate::from_bytes(
                            ::borsh::to_vec(
                                &#byte_display.parse_const::<#const_len>(#bytes_str).expect(format!("Parsing value {} as byte array failed", #bytes_str).as_str())
                            ).expect(format!("Borsh-encoding of value {} failed while encoding schema template on type {}", #bytes_str, #input_name_str).as_str())
                        )
                    }
                }
            };
            let template_name = t.0;
            // Return a Vec<(String, TransactionTemplate)> object (upon generated code evaluation)
            Ok(quote! { (#template_name.to_string(), #transaction_template) })
        }).collect::<Result<Vec<_>, darling::Error>>()?;

        // Finally, collect the results alongside an inserted call to recursively get the field type's own templates into an AttributeAndChildTemplateSet - see that type's
        // documentation for details
        Ok(quote! {
            #prefix::sov_universal_wallet::schema::transaction_templates::AttributeAndChildTemplateSet {
                attribute_templates: #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplateSet::fill_links(
                    vec![#(#templates),* ],
                    Self::get_child_links(#schema_arg),
                ),
                type_templates: <#ty as #prefix::sov_universal_wallet::schema::UniversalWallet>::get_child_templates(schema)
            }
        })
    }).collect::<Result<Vec<_>, darling::Error>>()?;

    Ok(quote! {
        #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplateSet::concatenate_template_sets(
            vec! [ #(#child_templates),* ],
            #input_name_str
        )
    })
}

/// Collect the tokens to implement one of the links returned as part of
/// UniversalWallet::get_child_links for a single variant of an enum type
fn enum_variant_child_link(
    type_ident: &Ident,
    generics: &Generics,
    prefix: &Option<syn::TypePath>,
) -> TokenStream {
    let ty_generics = generics.split_for_impl().1;
    quote! {
        <#type_ident #ty_generics as #prefix::sov_universal_wallet::schema::UniversalWallet>::make_linkable(schema)
    }
}

/// Collet the tokens to implement one of the links that will be fed to
/// TransactionTemplateSet::merge_enum_template_sets() that will make up the enum's own
/// implementation of get_child_templates
fn enum_variant_child_templates(
    type_ident: TokenStream,
    generics: &Generics,
    filter: &Vec<LitStr>,
    inherit: bool,
    prefix: &Option<syn::TypePath>,
) -> TokenStream {
    let ty_generics = generics.split_for_impl().1;
    let type_name = type_ident.to_string();
    quote! {
        <#type_ident #ty_generics as #prefix::sov_universal_wallet::schema::UniversalWallet>::get_child_templates(schema).filter_enum_variant_templates(vec![ #(#filter.to_string()),* ], #inherit, #type_name)
    }
}

fn extend_where_clause_with_field_bounds(
    fields: &[InputField],
    where_clause: &mut Option<WhereClause>,
    prefix: &Option<syn::TypePath>,
) {
    let output = where_clause.get_or_insert(WhereClause {
        where_token: Where::default(),
        predicates: Default::default(),
    });
    add_self_bound_to_where_clause(output);
    for field in fields.iter().filter(|field| !field.skip) {
        let ty = field.ty_tokens();
        let predicate = if let Some(bounds) = &field.bound {
            if bounds.0.is_empty() {
                continue;
            }
            syn::parse_quote! {
                #bounds
            }
        } else {
            generate_where_clause_simple_field_bound(&ty, prefix)
        };
        output.predicates.push(predicate);
        let template_bounds = generate_where_clause_template_parsing_bound(&field.template, &ty);
        output.predicates.extend(template_bounds);
    }
    if output.predicates.is_empty() {
        *where_clause = None
    }
}

fn add_self_bound_to_where_clause(clause: &mut WhereClause) {
    clause.predicates.push(syn::parse_quote! { Self: 'static });
}

fn generate_where_clause_template_parsing_bound(
    template: &TransactionTemplates,
    ty_tokens: &TokenStream,
) -> Vec<WherePredicate> {
    template
        .template
        .iter()
        .filter_map(|(_, iov)| match iov {
            InputOrValue::Input(_) | InputOrValue::FieldNameInput | InputOrValue::BytesValue(_) => {
                None
            }
            InputOrValue::Value(_) => Some(syn::parse_quote! {
                #ty_tokens: ::core::str::FromStr
            }),
            InputOrValue::DefaultValue => Some(syn::parse_quote! {
                #ty_tokens: ::core::default::Default
            }),
        })
        .collect()
}

fn generate_where_clause_simple_field_bound(
    ty_tokens: &TokenStream,
    prefix: &Option<syn::TypePath>,
) -> WherePredicate {
    syn::parse_quote! {
        #ty_tokens: #prefix::sov_universal_wallet::schema::UniversalWallet
    }
}

/// Main macro functionality. Implements the UniversalWallet trait on the given type, by
/// a) generating a scaffold containing Link::Placeholder links for every child,
/// b) collecting the types of every Placeholder child, in order, in get_child_links(), and
/// c) collecting any template construction information from attribute annotations (if any) into
/// get_child_templates()
///
/// Schema generation will then traverse the child link types, generate the schema for those and
/// fill in the Placeholder links from the scaffold in order.
///
/// The only two types of data that can be an input to the macro are a struct or an enum. Struct
/// type handling is relatively self-explanatory. Enums, however, can contain data inside their
/// variants as "virtual" structs or tuples; we reify these into corresponding physical structs,
/// on which we recursively derive UniversalWallet. This allows the enum variants to be scaffolded
/// with Placeholders, which will get filled in with normal links during schema building.
/// (The alternative would've been to generate a nested scaffold in-place for the
/// entire type, for every variant in the enum, considerably bloating individual type).
fn derive_wallet_field(
    input: DeriveInput,
    prefix: Option<syn::TypePath>,
    macro_name: TokenStream,
) -> Result<TokenStream, syn::Error> {
    let DeriveInput {
        ident, generics, ..
    } = &input;
    let input = Input::from_derive_input(&input)?;
    let input_name_str = ident.to_string();
    let hide_tag = input.hide_tag.unwrap_or_default();
    let template_tokens = quote_str_option_literally(&input.show_as);
    let child_templates_arg =
        Ident::from_string("schema").expect("Creating hardcoded identifier shouldn't fail");
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let mut where_clause = where_clause.cloned();
    let mut virtual_types: Vec<TokenStream> = vec![];
    let mut child_links: Vec<TokenStream> = vec![];
    let child_templates: TokenStream;
    let container = match &input.data {
        Data::Struct(s) => {
            if input.hide_tag.is_some() {
                return Err(syn::Error::new_spanned(
                    input.hide_tag,
                    "Only enums may be marked `hide_tag`",
                ));
            }
            if *input.template_inherit {
                return Err(syn::Error::new(
                    input.template_inherit.span(),
                    "The #[sov_wallet(template_inherit)] attribute can only be used on enums.",
                ));
            }
            child_links = struct_child_links(s, &prefix);
            child_templates =
                struct_child_templates(s, &child_templates_arg, &input_name_str, &prefix)?;
            match s.style {
                Style::Struct => build_struct_type_scaffold(
                    &s.fields,
                    input.ident.clone(),
                    input.attrs.serde,
                    template_tokens,
                    &mut where_clause,
                    &prefix,
                )?,
                Style::Tuple => build_tuple_type_scaffold(
                    &s.fields,
                    template_tokens,
                    &mut where_clause,
                    &prefix,
                )?,
                Style::Unit => {
                    let serde_name = input.attrs.serde.rename_typename(ident)?;
                    quote! {
                        #prefix::sov_universal_wallet::schema::Item::<#prefix::sov_universal_wallet::schema::IndexLinking>::Container(#prefix::sov_universal_wallet::schema::Container::Struct( #prefix::sov_universal_wallet::schema::container::StructWithSerde {
                            ty: #prefix::sov_universal_wallet::ty::Struct {
                                type_name: stringify!(#ident).to_string(),
                                template: #template_tokens,
                                peekable: false,
                                fields: vec![],
                            },
                            serde: #prefix::sov_universal_wallet::ty::ContainerSerdeMetadata {
                                name: #serde_name.to_string(),
                                fields_or_variants: vec![],
                            }
                        }))
                    }
                }
            }
        }
        Data::Enum(e) => {
            let where_under_construction = where_clause.get_or_insert(WhereClause {
                where_token: Where::default(),
                predicates: Default::default(),
            });
            let mut enum_child_templates: Vec<TokenStream> = Default::default();
            let mut current_discriminant: u8 = 0;
            let mut next_discriminant: u8 = 0;
            let inherit_variant_templates = *input.template_inherit;
            let (variants, metadatas) = e.iter()
            .map(|variant| {
                if next_discriminant < current_discriminant {
                    // That means we wrapped past 255 last loop. This variant has discriminant 256.
                    return Err(syn::Error::new_spanned(variant.ident.clone(), "Enums cannot have discriminants above 255. If explicit discriminants are used, ensure this does not result in a variant above this limit. If not, your enum must not have above 255 variants."));
                }
                current_discriminant = variant.discriminant(next_discriminant, input.attrs.borsh.use_discriminant)?;
                // We wrap here so we can detect and throw an error on the next variant. (We cannot
                // defer incrementing until the start of the next loop because the first variant
                // must be 0, so it can't be incremented at the start.)
                next_discriminant = current_discriminant.wrapping_add(1);

                let variant_ident = &variant.ident;
                let virtual_type_generics = virtual_field_generics(generics.clone(), &variant.fields.fields);
                let virtual_type_ident = virtual_typename(ident, variant_ident);
                let variant_showas = &variant.show_as;
                let variant_showas_tokens = quote_str_option_literally(variant_showas);
                let value = match &variant.fields.style {
                    Style::Struct => {
                        let virtual_struct = build_virtual_struct(&variant.fields.fields, where_under_construction, &virtual_type_ident, &virtual_type_generics, variant_showas, &prefix, &macro_name);
                        virtual_types.push(virtual_struct);
                        enum_child_templates.push(enum_variant_child_templates(variant.template_ty_tokens(&virtual_type_ident), &virtual_type_generics, &variant.template, inherit_variant_templates, &prefix));
                        child_links.push(enum_variant_child_link(&virtual_type_ident, &virtual_type_generics, &prefix));
                        quote!{ Some(#prefix::sov_universal_wallet::schema::Link::Placeholder) }
                    },
                    Style::Tuple => {
                        // TODO: Convert this to a warning and/or add an option to disable this check
                        if variant.fields.fields.len() > 1 && std::env::var("SOV_WALLET_PEDANTIC").is_ok() {
                            return Err(syn::Error::new_spanned(&input.ident, "Tuple structs with multiple entries are not human readable. Please use a named struct instead!"));
                        } else {
                            let virtual_tuple = build_virtual_tuple(&variant.fields.fields, where_under_construction, &virtual_type_ident, &virtual_type_generics, variant_showas, &prefix, &macro_name);
                            virtual_types.push(virtual_tuple);
                            enum_child_templates.push(enum_variant_child_templates(variant.template_ty_tokens(&virtual_type_ident), &virtual_type_generics, &variant.template, inherit_variant_templates, &prefix));
                            child_links.push(enum_variant_child_link(&virtual_type_ident, &virtual_type_generics, &prefix));
                            quote!{ Some(#prefix::sov_universal_wallet::schema::Link::Placeholder) }
                        }
                    },
                    Style::Unit => quote! { None },
                };
                let serde_variant_name = input.attrs.serde.rename_variant(variant_ident)?;

                Ok::<(TokenStream, TokenStream), syn::Error>(
                    (
                        quote! {
                            #prefix::sov_universal_wallet::ty::EnumVariant {
                                name: stringify!(#variant_ident).to_string(),
                                template: #variant_showas_tokens,
                                discriminant: #current_discriminant,
                                value: #value
                            }
                        },
                        quote! {
                            #prefix::sov_universal_wallet::ty::FieldOrVariantSerdeMetadata {
                                name: #serde_variant_name.to_string(),
                            }
                        }
                    )
                )})
                .collect::<Result<(Vec<_>, Vec<_>), _>>()?;
            add_self_bound_to_where_clause(where_under_construction);

            child_templates = quote! {
                #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplateSet::merge_enum_template_sets(
                    vec! [ #(#enum_child_templates),* ],
                    #input_name_str
                )
            };

            let serde_name = input.attrs.serde.rename_typename(ident)?;
            quote! {
                #prefix::sov_universal_wallet::schema::Item::<#prefix::sov_universal_wallet::schema::IndexLinking>::Container(#prefix::sov_universal_wallet::schema::Container::Enum(
                        #prefix::sov_universal_wallet::schema::container::EnumWithSerde {
                            ty: #prefix::sov_universal_wallet::ty::Enum {
                                type_name: stringify!(#ident).to_string(),
                                hide_tag: #hide_tag,
                                variants: vec![
                                    #(#variants),*
                                ],
                            },
                            serde: #prefix::sov_universal_wallet::ty::ContainerSerdeMetadata {
                                name: #serde_name.to_string(),
                                fields_or_variants: vec![
                                    #(#metadatas),*
                                ],
                            }
                        }
                ))
            }
        }
    };

    let schema = quote! {
        #[automatically_derived]
        impl #impl_generics #prefix::sov_universal_wallet::schema::UniversalWallet for #ident #ty_generics #where_clause {
            fn scaffold() -> #prefix::sov_universal_wallet::schema::Item::<#prefix::sov_universal_wallet::schema::IndexLinking> {
                #container
            }

            fn get_child_links(schema: &mut #prefix::sov_universal_wallet::schema::Schema) -> Vec<#prefix::sov_universal_wallet::schema::Link> {
                vec! [ #(#child_links),* ]
            }

            fn get_child_templates(#child_templates_arg: &mut #prefix::sov_universal_wallet::schema::Schema) -> #prefix::sov_universal_wallet::schema::transaction_templates::TransactionTemplateSet {
                #child_templates
            }
        }

        #(#virtual_types) *
    };

    Ok(schema)
}

/// Take a struct type and return the appropriate scaffold for it. The scaffold is just
/// ```text
/// Item::Container(Container::Struct(Struct {
///   name: "MyStruct",
///   fields: vec![
///    NammedField {
///      name: "some_field_name",
///      value: Link::Placeholder,
///      # ...
///    },
///    # ...
/// ]
/// }))
/// ```
pub fn build_struct_type_scaffold(
    fields: &[InputField],
    type_name: Ident,
    serde_rename: Serde,
    template_string: TokenStream,
    where_clause: &mut Option<WhereClause>,
    prefix: &Option<syn::TypePath>,
) -> Result<TokenStream, syn::Error> {
    extend_where_clause_with_field_bounds(fields, where_clause, prefix);

    let mut peekable = false;
    let (fields, field_serdes) = fields
        .iter()
        .filter(|field| !field.skip)
        .map(|field| {
            if let Some(FixedPointDisplay::FromField { field_index, .. }) = field.fixed_point {
                peekable = true;
                if *field_index >= fields.len() {
                    return Err(syn::Error::new(
                        field_index.span(),
                        "The field index referenced is out of bounds for this struct",
                    ));
                }
            }

            let name = field
                .ident
                .as_ref()
                .expect("Struct types have named fields");
            let doc = String::new(); // TODO
            let silent = field.hidden;
            let serde_name = serde_rename.rename_field(name)?;
            SynResult::<_>::Ok((
                quote! {
                    #prefix::sov_universal_wallet::ty::NamedField {
                        display_name: stringify!(#name).to_string(),
                        value: #prefix::sov_universal_wallet::schema::Link::Placeholder,
                        silent: #silent,
                        doc: #doc.to_string(),
                    }
                },
                quote! {
                    #prefix::sov_universal_wallet::ty::FieldOrVariantSerdeMetadata {
                        name: #serde_name.to_string()
                    }
                },
            ))
        })
        .collect::<Result<(Vec<_>, Vec<_>), _>>()?;

    let serde_name = serde_rename.rename_typename(&type_name)?;
    Ok(quote! {
        #prefix::sov_universal_wallet::schema::Item::<#prefix::sov_universal_wallet::schema::IndexLinking>::Container(#prefix::sov_universal_wallet::schema::Container::Struct( #prefix::sov_universal_wallet::schema::container::StructWithSerde {
            ty: #prefix::sov_universal_wallet::ty::Struct {
                type_name: stringify!(#type_name).to_string(),
                template: #template_string,
                peekable: #peekable,
                fields: vec![#(#fields),*],
            },
            serde: #prefix::sov_universal_wallet::ty::ContainerSerdeMetadata {
                name: #serde_name.to_string(),
                fields_or_variants: vec![#(#field_serdes),*],
            }
        }))
    })
}

/// Take a tuple type and return the appropriate scaffold for it. The scaffold is just
/// ```text
/// Item::Container(Container::Tuple(Tuple { fields: vec![
///    UnnamedField {
///      value: Link::Placeholder,
///      # ...
///    },
///    # ...
/// ]
/// }))
/// ```
pub fn build_tuple_type_scaffold(
    fields: &[InputField],
    template_string: TokenStream,
    where_clause: &mut Option<WhereClause>,
    prefix: &Option<syn::TypePath>,
) -> Result<TokenStream, syn::Error> {
    extend_where_clause_with_field_bounds(fields, where_clause, prefix);

    let fields = fields
        .iter()
        .filter(|field| !field.skip)
        .collect::<Vec<_>>();

    let mut peekable = false;

    let fields = fields
        .iter()
        .filter(|field| !field.skip)
        .map(|field| {
            if let Some(FixedPointDisplay::FromField { field_index, .. }) = field.fixed_point {
                peekable = true;
                if *field_index >= fields.len() {
                    return Err(syn::Error::new(
                        field_index.span(),
                        "The field index referenced is out of bounds for this struct",
                    ));
                }
            }

            let doc = String::new(); // TODO
            let silent = field.hidden;
            SynResult::<_>::Ok(quote! {
                #prefix::sov_universal_wallet::ty::UnnamedField {
                    value: #prefix::sov_universal_wallet::schema::Link::Placeholder,
                    silent: #silent,
                    doc: #doc.to_string(),
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(quote! {
        #prefix::sov_universal_wallet::schema::Item::<#prefix::sov_universal_wallet::schema::IndexLinking>::Container(#prefix::sov_universal_wallet::schema::Container::Tuple( #prefix::sov_universal_wallet::ty::Tuple {
            template: #template_string,
            peekable: #peekable,
            fields: vec![#(#fields),*],
        }))
    })
}

/// The name format for the generated types representing the contents of enum variants. Not meant
/// to be explicitly interacted with in user code or displayed to the user.
fn virtual_typename(enum_ident: &Ident, variant_ident: &Ident) -> Ident {
    format_ident!("__SovVirtualWallet_{}_{}", enum_ident, variant_ident)
}

/// When generating over an enum, filter the parent field's generics to those used in each variant.
/// Here `fields` are the subfields of the enum variant.
/// This is used to generate the virtual struct representing the variant's data, with the correct
/// generic bounds for the field.
fn virtual_field_generics(input_generics: Generics, fields: &[InputField]) -> Generics {
    let input_params = input_generics.declared_type_params();
    let input_lifetimes = input_generics.declared_lifetimes();
    let collected_generics = fields.collect_type_params(&Purpose::Declare.into(), &input_params);
    let collected_lifetimes = fields.collect_lifetimes(&Purpose::Declare.into(), &input_lifetimes);
    let mut virt_generics = input_generics.clone();
    virt_generics.params = virt_generics
        .params
        .into_iter()
        .filter(|gp| match gp {
            GenericParam::Type(t) => collected_generics.contains(&t.ident),
            GenericParam::Lifetime(l) => collected_lifetimes.contains(&l.lifetime),
            GenericParam::Const(_) => true,
        })
        .collect();
    if let Some(clause) = virt_generics.where_clause.as_mut() {
        clause.predicates = clause
            .predicates
            .clone()
            .into_iter()
            .filter(|p| match p {
                WherePredicate::Type(t) => syn_type_ident(&t.bounded_ty)
                    .map(|t| collected_generics.contains(t))
                    .unwrap_or(false),
                WherePredicate::Lifetime(l) => collected_lifetimes.contains(&l.lifetime),
                _ => false,
            })
            .collect();
    }
    virt_generics
}

/// Helper for the above. Where-clauses contain `Type` types, but darling's
/// `declared_type_params()` utility only returns `Ident`s - because that's the main information
/// available on a `TypeParam`, in fact - so the where-clauses have to be filtered based on the
/// `Ident`s of included types.
/// Thus we match on the very general `Type` enum and try to figure out if any where-clause bounds
/// involve any of the `TypeParam`s we know of; and for `Type`s which don't have a clear Ident we
/// default to None. This may not be a perfect heuristic but should cover the overwhelming majority
/// of normal Rust usage.
fn syn_type_ident(syn_type: &Type) -> Option<&Ident> {
    match syn_type {
        Type::Array(TypeArray { elem, .. })
        | Type::Group(TypeGroup { elem, .. })
        | Type::Paren(TypeParen { elem, .. })
        | Type::Ptr(TypePtr { elem, .. })
        | Type::Reference(TypeReference { elem, .. })
        | Type::Slice(TypeSlice { elem, .. }) => syn_type_ident(elem),
        Type::Path(t) => t.path.segments.first().map(|s| &s.ident),
        _ => None,
    }
}

/// Build an explicit struct type from the fields inside an enum variant.
/// This is a relatively simple wrapper around the variant's fields, the only extra complexity lies
/// in generating the correct bounds for the union of the generics used by that variant's fields.
///
/// Crucially, we then recursively #[derive(UniversalWallet)] on it.
/// This has an important consequence for hygiene, as the crate does not define the macro, so we
/// require that `UniversalWallet` be defined and in scope.
fn build_virtual_struct(
    fields: &[InputField],
    where_clause: &mut WhereClause,
    type_name: &Ident,
    type_generics: &Generics,
    showas_string: &Option<String>,
    prefix: &Option<syn::TypePath>,
    macro_name: &TokenStream,
) -> TokenStream {
    let (virt_impl_generics, virt_ty_generics, virt_where_clause) = type_generics.split_for_impl();
    where_clause
        .predicates
        .push(generate_where_clause_simple_field_bound(
            &quote! { #type_name #virt_ty_generics },
            prefix,
        ));
    let struct_fields: Vec<_> = fields
        .iter()
        .map(|field| {
            let name = &field.ident;
            let field_type = field.ty_tokens();
            let bounds_attribute = field.bound.as_ref().map(|b| {
                let tokens = format!("{}", b.to_token_stream());
                quote! {
                    #[sov_wallet(bound=#tokens)]
                }
            });
            let templates = field.template.original.clone();
            let template_attribute = if templates.is_empty() {
                quote! {}
            } else {
                quote! {#[sov_wallet(#templates)]}
            };
            let template_override_attribute = match &field.template_override_ty {
                Some(ty) => quote!{#[sov_wallet(template_override_ty = #ty)]},
                None => quote!{}
            };
            // TODO: propagate `#[serde(rename)]` attributes to each field here if support for it
            // is required
            quote! {
                #template_attribute #template_override_attribute #bounds_attribute #name: #field_type
            }
        })
        .collect::<Vec<_>>();

    // TODO: pass attribute name as argument when creating the derive, instead of hardcoding
    // sov_wallet
    let showas_attribute = match showas_string {
        Some(template) => quote! {#[sov_wallet(show_as = #template)]},
        None => quote! {},
    };

    // TODO: propagate `#[serde(rename_all_fields)]` here, transformed into a `rename_all`,
    // if support for that is required
    let virtual_struct = quote! {
        #[allow(non_camel_case_types, dead_code)]
        #[automatically_derived]
        #[derive(#macro_name)]
        #showas_attribute
        struct #type_name #virt_impl_generics #virt_where_clause {
            #(#struct_fields),*
        }
    };

    virtual_struct
}

/// Build an explicit tuple struct type from the fields inside an enum variant.
/// This is a relatively simple wrapper around the variant's fields, the only extra complexity lies
/// in generating the correct bounds for the union of the generics used by that variant's fields.
///
/// Crucially, we then recursively #[derive(UniversalWallet)] on it.
/// This has an important consequence for hygiene, as the crate does not define the macro, so we
/// require that `UniversalWallet` be defined and in scope.
fn build_virtual_tuple(
    fields: &[InputField],
    where_clause: &mut WhereClause,
    type_name: &Ident,
    type_generics: &Generics,
    template_string: &Option<String>,
    prefix: &Option<syn::TypePath>,
    macro_name: &TokenStream,
) -> TokenStream {
    let (virt_impl_generics, virt_ty_generics, virt_where_clause) = type_generics.split_for_impl();
    where_clause
        .predicates
        .push(generate_where_clause_simple_field_bound(
            &quote! { #type_name #virt_ty_generics },
            prefix,
        ));
    let tuple_fields: Vec<_> = fields
        .iter()
        .map(|field| {
            let field_type = &field.ty;
            let bounds_attribute = field.bound.as_ref().map(|b| {
                let tokens = format!("{}", b.to_token_stream());
                quote! {
                    #[sov_wallet(bound=#tokens)]
                }
            });
            let templates = field.template.original.clone();
            let template_attribute = if templates.is_empty() {
                quote! {}
            } else {
                quote! {#[sov_wallet(#templates)]}
            };
            let template_override_attribute = match &field.template_override_ty {
                Some(ty) => quote! {#[sov_wallet(template_override_ty = #ty)]},
                None => quote! {},
            };
            quote! {
                #template_attribute #template_override_attribute #bounds_attribute #field_type
            }
        })
        .collect::<Vec<_>>();

    // TODO: pass attribute name as argument when creating the derive, instead of hardcoding
    // sov_wallet
    let template_attribute = match template_string {
        Some(template) => quote! {#[sov_wallet(show_as = #template)]},
        None => quote! {},
    };

    // TODOs as above for build_virtual_struct
    let virtual_tuple = quote! {
        #[allow(non_camel_case_types, dead_code)]
        #[automatically_derived]
        #[derive(#macro_name)]
        #template_attribute
        struct #type_name #virt_impl_generics (
            #(#tuple_fields),*
        ) #virt_where_clause;
    };

    virtual_tuple
}

fn maybe_generate_field_schema(field: &InputField, prefix: &Option<syn::TypePath>) -> TokenStream {
    field.resolve_type(prefix)
}

/// necessary to have quote! actually include the Option tokens
/// String options get quoted as a static str, so instead of a generic util this is a simple
/// special-cased helper due to the need to call .to_string()
fn quote_str_option_literally(opt: &Option<String>) -> TokenStream {
    match opt {
        Some(s) => quote! { Some(#s.to_string()) },
        None => quote! { None },
    }
}

#[derive(Debug, FromDeriveInput)]
#[darling(
    attributes(sov_wallet),
    supports(any),
    forward_attrs(doc, serde, borsh)
)]
pub struct Input {
    pub ident: Ident,
    pub data: ast::Data<InputVariant, InputField>,
    #[darling(with = "parse_foreign_attrs")]
    pub attrs: ForeignAttrs,
    #[darling(default)]
    pub show_as: Option<String>,
    #[darling(default)]
    pub hide_tag: Option<bool>,
    #[darling(default)]
    pub template_inherit: SpannedValue<bool>,
}

#[derive(Debug, Clone, FromField)]
#[darling(attributes(sov_wallet), forward_attrs(doc, serde))]
pub struct InputField {
    pub attrs: Vec<syn::Attribute>,
    pub ident: Option<Ident>,
    pub ty: syn::Type,
    #[darling(default)]
    pub skip: bool,
    #[darling(default)]
    pub display: Option<ByteDisplayType>,
    #[darling(default)]
    pub fixed_point: Option<FixedPointDisplay>,
    #[darling(default)]
    pub bound: Option<Bounds>,
    #[darling(default)]
    pub template: TransactionTemplates,
    #[darling(default)]
    pub template_override_ty: Option<syn::Type>,
    #[darling(default)]
    pub hidden: bool,
    #[darling(default, rename = "as_ty")]
    pub as_ty: Option<syn::Type>,
}

uses_type_params!(InputField, ty);
uses_lifetimes!(InputField, ty);

impl InputField {
    pub fn ty_tokens(&self) -> TokenStream {
        if let Some(ty) = &self.as_ty {
            quote! { #ty }
        } else {
            let ty = &self.ty;
            quote! { #ty }
        }
    }

    pub fn resolve_type(&self, crate_prefix: &Option<syn::TypePath>) -> TokenStream {
        let ty = self.ty_tokens();
        if let Some(display) = &self.display {
            let display_tokens = display.resolve(crate_prefix);
            quote! {
                {
                    <#ty as #crate_prefix::sov_universal_wallet::ty::ByteDisplayable>::with_display(#display_tokens)
                }
            }
        } else if let Some(display) = &self.fixed_point {
            let display_tokens = display.resolve(crate_prefix);
            quote! {
                {
                    <#ty as #crate_prefix::sov_universal_wallet::ty::IntegerDisplayable>::with_display(#display_tokens)
                }
            }
        } else {
            quote! {
                <#ty as #crate_prefix::sov_universal_wallet::schema::UniversalWallet>::make_linkable(schema)
            }
        }
    }
}

#[derive(Debug, Clone, FromVariant)]
#[darling(attributes(sov_wallet), forward_attrs(doc))]
pub struct InputVariant {
    pub ident: Ident,
    pub fields: ast::Fields<InputField>,
    pub discriminant: Option<Expr>,
    #[darling(default)]
    pub show_as: Option<String>,
    #[darling(default)]
    pub template: Vec<LitStr>,
    #[darling(default)]
    pub template_override_ty: Option<syn::Type>,
}

impl InputVariant {
    pub fn template_ty_tokens(&self, virtual_typename: &Ident) -> TokenStream {
        match &self.template_override_ty {
            Some(ty) => ty.to_token_stream(),
            None => virtual_typename.to_token_stream(),
        }
    }

    pub fn discriminant(
        &self,
        current_counter: u8,
        use_discriminant: bool,
    ) -> Result<u8, darling::Error> {
        if let Some(explicit_discriminant) = &self.discriminant {
            if use_discriminant {
                return <u8 as FromMeta>::from_expr(explicit_discriminant);
            }
        }
        Ok(current_counter)
    }
}
