use quote::{format_ident, quote};
use syn::{Data, DataEnum, DeriveInput, Fields, Ident, Type};

use crate::common::{extract_ident, generics_for_field, StructFieldExtractor};

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
    ) -> Result<proc_macro::TokenStream, syn::Error> {
        let DeriveInput {
            ident,
            generics,
            data,
            ..
        } = input;
        let fields = self.field_extractor.get_fields_from_struct(&data)?;

        let (_, ty_generics, _) = generics.split_for_impl();

        let mut module_command_arms = vec![];
        let mut module_args = vec![];
        let mut match_arms = vec![];
        let mut parse_match_arms = vec![];
        let mut deserialize_constraints: Vec<syn::WherePredicate> = vec![];

        // Loop over the fields
        'outer: for field in &fields {
            // Skip fields with the attribute cli_skip
            for attr in field.attrs.iter() {
                if attr.path.is_ident("cli_skip") {
                    continue 'outer;
                }
            }

            // For each type path we encounter, we need to extract the generic type parameters for that field
            // and construct a `Generics` struct that contains the bounds for each of those generic type parameters.
            if let syn::Type::Path(type_path) = &field.ty {
                let mut module_path = type_path.path.clone();
                if let Some(segment) = module_path.segments.last_mut() {
                    let field_generic_types = &segment.arguments;
                    let field_generics_with_bounds =
                        generics_for_field(&generics, field_generic_types);

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
                    let _field_name_string = field_name.to_string();

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
                                <#ident:: #ty_generics as sov_data_generators::EncodeCall<#module_path>> ::encode_call(
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
                                let _data: <#module_path as ::sov_modules_api::Module>::CallMessage =
                                 ::serde_json::from_str::<<#module_path as ::sov_modules_api::Module>::CallMessage>(&call_data)?;
                                <#ident:: #ty_generics as sov_data_generators::EncodeCall<#module_path>> ::encode_call(
                                   _data
                                )
                            }),
                        });
                }
            }
        }

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
            /// A CLI parser for transactions which can be sent to the runtime
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

pub(crate) fn derive_cli_wallet_arg(
    ast: DeriveInput,
) -> Result<proc_macro::TokenStream, syn::Error> {
    let item_name = &ast.ident;
    let generics = &ast.generics;
    let item_with_named_fields_ident = Ident::new(
        &format!("{}WithNamedFields", item_name),
        proc_macro2::Span::call_site(),
    );
    let is_generic = !generics.params.is_empty();

    let (named_type_defn, conversion_logic) = match &ast.data {
        // Creating an enum "_WithNamedFields" which is identical to the first enum
        // except that all fields are named.
        Data::Enum(DataEnum { variants, .. }) => {
            // For each variant of the enum, we have to specify two things:
            //   1. The structure of the "named" version of the variant
            //   2. How to convert from the "named" version of the variant to the original version
            let mut variants_with_named_fields = vec![];
            let mut convert_cases = vec![];

            for variant in variants {
                let variant_name = &variant.ident;
                let variant_docs = variant
                    .attrs
                    .iter()
                    .filter(|attr| attr.path.is_ident("doc"))
                    .collect::<Vec<_>>();

                let mut named_variant_fields =
                    StructFieldExtractor::get_or_generate_named_fields(&variant.fields);
                named_variant_fields
                    .iter_mut()
                    .for_each(|field| field.filter_attrs(|attr| attr.path.is_ident("doc")));
                let variant_field_names = named_variant_fields
                    .iter()
                    .map(|f| &f.ident)
                    .collect::<Vec<_>>();

                match &variant.fields {
                    Fields::Unnamed(_) => {
                        variants_with_named_fields.push(quote! {
                            #( #variant_docs )*
                            #[command(arg_required_else_help(true))]
                            #variant_name {#(#named_variant_fields),* }
                        });
                        convert_cases.push(quote! {
                            #item_with_named_fields_ident::#variant_name {#(#variant_field_names),*} => #item_name::#variant_name(#(#variant_field_names),*),
                        });
                    }
                    Fields::Named(_) => {
                        variants_with_named_fields.push(quote! {
                            #( #variant_docs )*
                            #[command(arg_required_else_help(true))]
                            #variant_name {#(#named_variant_fields),* }
                        });
                        convert_cases.push(quote! {
                        #item_with_named_fields_ident::#variant_name {#(#variant_field_names),*} => #item_name::#variant_name {#(#variant_field_names),*},
                    });
                    }
                    Fields::Unit => {
                        variants_with_named_fields.push(quote! {
                            #( #variant_docs )*
                            #variant_name
                        });
                        convert_cases.push(quote! {
                            #item_with_named_fields_ident::#variant_name => #item_name::#variant_name,
                        });
                    }
                }
            }

            let enum_defn = quote! {
                #[derive(::clap::Parser)]
                pub enum #item_with_named_fields_ident #generics {
                    #(#variants_with_named_fields,)*
                }
            };

            let from_body = quote! {
                match item {
                    #(#convert_cases)*
                }
            };
            (enum_defn, from_body)
        }
        Data::Struct(s) => {
            let mut named_fields = StructFieldExtractor::get_or_generate_named_fields(&s.fields);
            named_fields
                .iter_mut()
                .for_each(|field| field.filter_attrs(|attr| attr.path.is_ident("doc")));
            let field_names = named_fields.iter().map(|f| &f.ident).collect::<Vec<_>>();
            let conversion_logic = match s.fields {
                Fields::Named(_) => quote! {{
                        let #item_with_named_fields_ident { #(#field_names),* } = item;
                        #item_name{#(#field_names),*}
                }},
                Fields::Unnamed(_) => {
                    quote! {
                            let #item_with_named_fields_ident { #(#field_names),* } = item;
                            #item_name(#(#field_names),*)
                    }
                }
                Fields::Unit => quote! { #item_name },
            };

            let struct_defn = quote! {
                #[derive(::clap::Parser)]
                pub struct #item_with_named_fields_ident #generics {
                    #(#named_fields),*
                }
            };
            (struct_defn, conversion_logic)
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                ast,
                "Unions are not supported as CLI wallet args",
            ))
        }
    };

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let clap_type = if is_generic {
        quote! { #item_with_named_fields_ident #ty_generics }
    } else {
        quote! { #item_with_named_fields_ident }
    };

    let expanded = quote! {
        // Create a new data type which matches the original, but with all fields named.
        // This is the type that Clap will parse the CLI args into
        #named_type_defn

        // Define a `From` implementation which converts from the named fields version to the original version
        impl #impl_generics From<#clap_type> for #item_name #ty_generics #where_clause {
            fn from(item: #item_with_named_fields_ident #ty_generics) -> Self {
                #conversion_logic
            }
        }

        // Implement the `CliWalletArg` trait for the original type. This is what allows the original type to be used as a CLI arg.
        impl #impl_generics sov_modules_api::CliWalletArg for #item_name #ty_generics #where_clause {
            type CliStringRepr = #clap_type;
        }
    };
    Ok(expanded.into())
}
