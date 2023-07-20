use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Data, DataEnum, DeriveInput, Fields, GenericParam, Ident, PathArguments, Type};

use crate::common::{extract_generic_type_bounds, extract_ident, StructFieldExtractor};

pub(crate) struct CliParserMacro {
    field_extractor: StructFieldExtractor,
}

impl CliParserMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn cli_macro(
        &self,
        input: DeriveInput,
        skip_fields: Vec<String>,
    ) -> Result<proc_macro::TokenStream, syn::Error> {
        let DeriveInput {
            attrs,
            vis,
            ident,
            generics,
            data,
        } = input;
        let fields = self.field_extractor.get_fields_from_struct(&data)?;
        let generic_bounds = extract_generic_type_bounds(&generics);

        // We assume that the `Context` type is the first generic type parameter
        // Since macro expansion happens before type inference, there is no reliable way
        // to extract the `Context` type without making this assumption. (i.e. we can't look for a type
        // that implements `sov_modules_api::Context`, because that name might have been aliased)
        let context_type = generics
            .params
            .iter()
            .find_map(|item| {
                if let GenericParam::Type(type_param) = item {
                    Some(type_param)
                } else {
                    None
                }
            })
            .ok_or(syn::Error::new_spanned(
                &generics,
                "a runtime must be generic over a sov_modules_api::Context to derive cli_parser",
            ))?
            .ident
            .clone();

        let mut module_command_arms = vec![];
        let mut module_args = vec![];
        let mut match_arms = vec![];
        let mut parse_match_arms = vec![];
        let mut deserialize_constraints: Vec<syn::WherePredicate> = vec![];

        // Loop over the fields
        for field in &fields {
            if skip_fields.contains(&field.ident.to_string()) {
                continue;
            }
            // For each type path we encounter, we need to extract the generic type parameters for that field
            // and construct a `Generics` struct that contains the bounds for each of those generic type parameters.
            if let syn::Type::Path(type_path) = &field.ty {
                let mut module_path = type_path.path.clone();
                if let Some(segment) = module_path.segments.last_mut() {
                    let field_generic_types = &segment.arguments;
                    let field_generics_with_bounds = match field_generic_types {
                        PathArguments::AngleBracketed(angle_bracketed_data) => {
                            let mut args_with_bounds =
                                Punctuated::<GenericParam, syn::token::Comma>::new();
                            for generic_arg in &angle_bracketed_data.args {
                                if let syn::GenericArgument::Type(syn::Type::Path(type_path)) =
                                    generic_arg
                                {
                                    let ident = extract_ident(type_path);
                                    let bounds =
                                        generic_bounds.get(type_path).cloned().unwrap_or_default();

                                    // Construct a "type param" with the appropriate bounds. This corresponds to a syntax
                                    // tree like `T: Trait1 + Trait2`
                                    let generic_type_param_with_bounds = syn::TypeParam {
                                        attrs: Vec::new(),
                                        ident: ident.clone(),
                                        colon_token: Some(syn::token::Colon {
                                            spans: [type_path.span()],
                                        }),
                                        bounds: bounds.clone(),
                                        eq_token: None,
                                        default: None,
                                    };
                                    args_with_bounds
                                        .push(GenericParam::Type(generic_type_param_with_bounds))
                                }
                            }
                            // Construct a `Generics` struct with the generic type parameters and their bounds.
                            // This corresponds to a syntax tree like `<T: Trait1 + Trait2>`
                            syn::Generics {
                                lt_token: Some(syn::token::Lt {
                                    spans: [type_path.span()],
                                }),
                                params: args_with_bounds,
                                gt_token: Some(syn::token::Gt {
                                    spans: [type_path.span()],
                                }),
                                where_clause: None,
                            }
                        }
                        // We don't need to do anything if the generic type parameters are not angle bracketed
                        _ => Default::default(),
                    };

                    let module_ident = segment.ident.clone();
                    let module_args_ident = format_ident!("{}Args", module_ident);
                    module_command_arms.push(quote! {
                        #module_ident(#module_args_ident #field_generic_types)
                    });
                    module_args.push(quote! {
                        #[derive(::clap::Parser)]
                        pub struct #module_args_ident #field_generics_with_bounds {
                            #[clap(subcommand)]
                            /// Commands under #module
                            command: <<#module_path as ::sov_modules_api::Module>::CallMessage as ::sov_modules_api::CliWalletArg>::CliStringRepr,
                        }
                    });

                    let field_name = field.ident.clone();
                    let field_name_string = field_name.to_string();
                    let encode_function_name = format_ident!("encode_{}_call", field_name_string);

                    let type_name_string = match &field.ty {
                        Type::Path(type_path) => extract_ident(type_path).to_string(),
                        _ => {
                            return Err(syn::Error::new_spanned(
                                field.ident.clone(),
                                "expected a type path",
                            ))
                        }
                    };

                    // Build the `match` arm for the CLI's `clap` parse function
                    parse_match_arms.push(quote! {
                            CliTransactionParser::#module_ident(mod_args) => {
                                let command_as_call_message: <#module_path as ::sov_modules_api::Module>::CallMessage = mod_args.command.into();
                                #ident::<#context_type>::#encode_function_name(
                                    command_as_call_message
                                )
                            },
                         });

                    // Build a constraint requiring that all call messages support serde deserialization
                    let deserialization_constraint = {
                        let type_path: syn::TypePath = syn::parse_quote! {<#module_path as ::sov_modules_api::Module>::CallMessage };
                        let bounds: syn::TypeParamBound =
                            syn::parse_quote! {::serde::de::DeserializeOwned};
                        syn::WherePredicate::Type(syn::PredicateType {
                            lifetimes: None,
                            bounded_ty: syn::Type::Path(type_path),
                            colon_token: Default::default(),
                            bounds: vec![bounds].into_iter().collect(),
                        })
                    };
                    deserialize_constraints.push(deserialization_constraint);

                    // Build the `match` arms for the CLI's json parser
                    match_arms.push(quote! {
                            #type_name_string => Ok({
                                #ident::<#context_type>::#encode_function_name(
                                    ::serde_json::from_str::<<#module_path as ::sov_modules_api::Module>::CallMessage>(&call_data)?
                                )
                            }),
                        });
                }
            }
        }

        // Create tokens for original struct fields
        let original_struct_fields: Vec<_> = fields
            .into_iter()
            .map(|field| {
                let field_name = field.ident;
                let field_type = field.ty;
                let field_vis = field.vis;

                quote! {
                    #field_vis #field_name: #field_type
                }
            })
            .collect();

        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
        let where_clause_with_deserialize_bounds = match where_clause {
            Some(where_clause) => {
                let mut result = where_clause.clone();
                result
                    .predicates
                    .extend(deserialize_constraints.into_iter());
                result
            }
            None => syn::parse_quote! {
                where #(#deserialize_constraints),*
            },
        };
        // Merge and generate the new code
        let expanded = quote! {
            // re-declare the original struct
            #(#attrs)*
            #vis struct #ident #generics {
                #(#original_struct_fields),*
            }

            // generate the rest of the code
            // #( #command_types )*
            /// List of utility commands
            #[derive(::clap::Parser)]
            pub enum CliTransactionParser #generics {
                #( #module_command_arms, )*
            }
            #( #module_args )*

            /// Borsh encode a transaction parsed from the CLI
            pub fn borsh_encode_cli_tx #impl_generics (cmd: CliTransactionParser #ty_generics) -> ::std::vec::Vec<u8>
            #where_clause {
                use ::borsh::BorshSerialize;
                match cmd {
                    #(#parse_match_arms)*
                    _ => panic!("unknown module name"),
                }
            }

            /// Attempts to parse the provided call data as a [`sov_modules_api::Module::CallMessage`] for the given module.
            pub fn parse_call_message_json #impl_generics (module_name: &str, call_data: &str) -> ::anyhow::Result<Vec<u8>>
            #where_clause_with_deserialize_bounds
             {
                match module_name {
                    #(#match_arms)*
                    _ => panic!("unknown module name"),
                }
            }
        };

        Ok(expanded.into())
    }
}

/// Derive [`clap::Parser`] for an enum with unnamed fields.
///
/// Under the hood, this is done by generating an identical enum with dummy field names, then deriving [`clap::Parser`] for that enum.
pub fn derive_clap_custom_enum(ast: DeriveInput) -> Result<proc_macro::TokenStream, syn::Error> {
    let enum_name = &ast.ident;
    let generics = &ast.generics;

    if let Data::Enum(DataEnum { variants, .. }) = ast.data.clone() {
        let mut variants_with_fields = vec![];
        let mut convert_cases = vec![];

        let cli_enum_with_fields_ident = Ident::new(
            &format!("{}WithNamedFields", enum_name),
            proc_macro2::Span::call_site(),
        );

        let is_generic = !generics.params.is_empty();

        for variant in variants {
            let ident = &variant.ident;
            let doc_attrs_variant = variant
                .attrs
                .iter()
                .filter(|attr| attr.path.is_ident("doc"))
                .collect::<Vec<_>>();

            match &variant.fields {
                Fields::Unnamed(unnamed_fields) => {
                    let named_fields = unnamed_fields
                        .unnamed
                        .iter()
                        .enumerate()
                        .map(|(i, field)| {
                            let name = Ident::new(&format!("field{}", i), field.span());
                            let ty = &field.ty;
                            let doc_attrs_field = field
                                .attrs
                                .iter()
                                .filter(|attr| attr.path.is_ident("doc"))
                                .collect::<Vec<_>>();
                            quote! {
                                #( #doc_attrs_field )*
                                #name: #ty
                            }
                        })
                        .collect::<Vec<_>>();
                    variants_with_fields.push(quote! {
                        #( #doc_attrs_variant )*
                        #ident {#(#named_fields),*}
                    });

                    let fields = unnamed_fields
                        .unnamed
                        .iter()
                        .enumerate()
                        .map(|(i, _)| {
                            let name = Ident::new(&format!("field{}", i), unnamed_fields.span());
                            quote! {#name}
                        })
                        .collect::<Vec<_>>();

                    convert_cases.push(quote! {
                        #cli_enum_with_fields_ident::#ident {#(#fields),*} => #enum_name::#ident(#(#fields),*),
                    });
                }
                Fields::Named(fields_named) => {
                    let field_tokens = fields_named
                        .named
                        .iter()
                        .map(|field| {
                            let name = field.ident.as_ref().unwrap();
                            let ty = &field.ty;
                            let doc_attrs_field = field
                                .attrs
                                .iter()
                                .filter(|attr| attr.path.is_ident("doc"))
                                .collect::<Vec<_>>();
                            quote! {
                                #( #doc_attrs_field )*
                                #name: #ty
                            }
                        })
                        .collect::<Vec<_>>();

                    variants_with_fields.push(quote! {
                        #( #doc_attrs_variant )*
                        #ident {#(#field_tokens),*}
                    });

                    let fields = fields_named
                        .named
                        .iter()
                        .map(|field| {
                            let name = field.ident.as_ref().unwrap();
                            quote! {#name}
                        })
                        .collect::<Vec<_>>();

                    convert_cases.push(quote! {
                        #cli_enum_with_fields_ident::#ident {#(#fields),*} => #enum_name::#ident {#(#fields),*},
                    });
                }
                Fields::Unit => {
                    variants_with_fields.push(quote! {
                        #( #doc_attrs_variant )*
                        #ident
                    });
                    convert_cases.push(quote! {
                        #cli_enum_with_fields_ident::#ident => #enum_name::#ident,
                    });
                }
            }
        }

        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        let clap_type = if is_generic {
            quote! { #cli_enum_with_fields_ident #ty_generics }
        } else {
            quote! { #cli_enum_with_fields_ident }
        };

        let expanded = quote! {
            #[derive(::clap::Parser)]
            pub enum #cli_enum_with_fields_ident #generics {
                #(#variants_with_fields,)*
            }

            impl #impl_generics From<#clap_type> for #enum_name #ty_generics #where_clause {
                fn from(item: #cli_enum_with_fields_ident #ty_generics) -> Self {
                    match item {
                        #(#convert_cases)*
                    }
                }
            }

            impl #impl_generics sov_modules_api::CliWalletArg for #enum_name #ty_generics #where_clause {
                type CliStringRepr = #clap_type;
            }
        };

        Ok(expanded.into())
    } else {
        panic!("This derive macro only works with enums");
    }
}
