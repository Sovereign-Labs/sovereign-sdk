use quote::{format_ident, quote};
use syn::{Data, DataEnum, DeriveInput, Fields, Ident};

use crate::common::StructFieldExtractor;

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

        let mut module_json_parser_arms = vec![];
        let mut module_message_arms = vec![];
        let mut try_from_subcommand_match_arms = vec![];
        let mut try_map_match_arms = vec![];
        let mut from_json_match_arms = vec![];
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
                let module_path = type_path.path.clone();
                let field_name = field.ident.clone();
                let doc_str = format!("A subcommand for the `{}` module", &field_name);
                let doc_contents = format!("A clap argument for the `{}` module", &field_name);

                module_json_parser_arms.push(quote! {
                    #[doc = #doc_str]
                    #field_name {
                        #[doc = #doc_contents]
                        #[clap(flatten)]
                        contents: __Inner
                    }
                });

                module_message_arms.push(quote! {
                    #[doc = #doc_str]
                    #field_name {
                        #[doc = #doc_contents]
                        contents: __Inner
                    }
                });

                from_json_match_arms.push(quote! {
                    RuntimeMessage::#field_name{ contents } => {
                                ::serde_json::from_str::<<#module_path as ::sov_modules_api::Module>::CallMessage>(&contents.json).map(
                                    // Use the enum variant as a constructor
                                    <#ident #ty_generics as ::sov_modules_api::DispatchCall>::Decodable:: #field_name
                                )
                            },
                         });

                try_map_match_arms.push(quote! {
                    RuntimeMessage::#field_name { contents } => RuntimeMessage::#field_name { contents: contents.try_into()? },
                });

                try_from_subcommand_match_arms.push(quote! {
                    RuntimeSubcommand::#field_name { contents } => RuntimeMessage::#field_name { contents: contents.try_into()? },
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
            }
        }

        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        let where_clause_with_deserialize_bounds = match where_clause {
            Some(where_clause) => {
                let mut result = where_clause.clone();
                result.predicates.extend(deserialize_constraints);
                result
            }
            None => syn::parse_quote! {
                where #(#deserialize_constraints),*
            },
        };

        // The generics from the `runtime`, with an additional `__Inner` generic
        // which holds the clap arguments.
        let generics_with_inner = {
            let mut generics = generics.clone();
            generics.params.insert(0, syn::parse_quote! {__Inner });
            generics.where_clause = match generics.where_clause {
                Some(where_clause) => {
                    let mut result = where_clause;
                    result
                        .predicates
                        .push(syn::parse_quote! { __Inner: ::clap::Args });
                    Some(result)
                }
                None => syn::parse_quote! {
                    where  __Inner: ::clap::Args
                },
            };
            generics
        };
        let (impl_generics_with_inner, ty_generics_with_inner, where_clause_with_inner_as_clap) =
            generics_with_inner.split_for_impl();

        // Generics identical to generics_with_inner, but with the `__Inner` type renamed to `__Dest`.
        // This type is used in the the try_map conversion
        let generics_for_dest = {
            let mut generics = generics.clone();
            generics.params.insert(0, syn::parse_quote! {__Dest});
            generics
        };
        let (_, ty_generics_for_dest, _) = generics_for_dest.split_for_impl();

        let generics_with_inner_and_dest = {
            let mut generics = generics_with_inner.clone();
            generics.params.insert(0, syn::parse_quote! {__Dest});
            if let Some(c) = generics.where_clause.as_mut() {
                c.predicates
                    .push(syn::parse_quote! { __Dest: ::core::convert::TryFrom<__Inner> })
            }
            generics
        };
        let (impl_generics_with_inner_and_dest, _, where_clause_with_inner_clap_and_try_from) =
            generics_with_inner_and_dest.split_for_impl();

        // Generics identical to `generics_with_inner`, with the `__Inner` type bound to `JsonStringArg`
        let generics_for_json = {
            let mut generics = generics.clone();
            generics
                .params
                .insert(0, syn::parse_quote! {__JsonStringArg});
            generics
        };
        let (_impl_generics_for_json, ty_generics_for_json, _) = generics_for_json.split_for_impl();

        // Merge and generate the new code
        let expanded = quote! {


            /// An enum expressing the subcommands available to this runtime. Contains
            /// one subcommand for each module, except modules annotated with the #[cli_skip] attribute
            #[derive(::clap::Parser)]
            #[allow(non_camel_case_types)]
            pub enum RuntimeSubcommand #impl_generics_with_inner #where_clause_with_inner_as_clap {
                #( #module_json_parser_arms, )*
                #[clap(skip)]
                #[doc(hidden)]
                ____phantom(::std::marker::PhantomData<#ident #ty_generics>)
            }

            impl #impl_generics_with_inner ::sov_modules_api::cli::CliFrontEnd<#ident #ty_generics> for RuntimeSubcommand #ty_generics_with_inner #where_clause_with_deserialize_bounds, __Inner: ::clap::Args {
                type CliIntermediateRepr<__Dest> = RuntimeMessage #ty_generics_for_dest;
            }

            /// An intermediate enum between the RuntimeSubcommand (which must implement `clap`) and the
            /// final RT::Decodable type. Like the RuntimeSubcommand, this type contains one variant for each cli-enabled module.
            #[allow(non_camel_case_types)]
            pub enum RuntimeMessage #impl_generics_with_inner #where_clause {
                #( #module_message_arms, )*
                #[doc(hidden)]
                ____phantom(::std::marker::PhantomData<#ident #ty_generics>)
            }

            use ::sov_modules_api::cli::JsonStringArg as __JsonStringArg;
            // Implement TryFrom<RuntimeMessage<JsonStringArg>> for the runtime's call message. Uses serde_json to deserialize the json string.
            impl #impl_generics ::core::convert::TryFrom<RuntimeMessage #ty_generics_for_json> for <#ident #ty_generics as ::sov_modules_api::DispatchCall>::Decodable #where_clause_with_deserialize_bounds {
                type Error = ::serde_json::Error;
                fn try_from(item: RuntimeMessage #ty_generics_for_json ) -> Result<Self, Self::Error> {
                    match item {
                        #( #from_json_match_arms )*
                        RuntimeMessage::____phantom(_) => unreachable!(),
                    }
                }
            }

            // Allow arbitrary conversions from the `clap`-enabled `RuntimeSubcommand` to the less constrained `RuntimeMessage` enum.
            // This allows us to (for example), accept a `JsonStringArgs` or a `FileNameArgs` as a CLI argument, and then
            // use fallible logic to convert it into the final JSON string to be parsed into a callmessage.
            impl #impl_generics_with_inner_and_dest ::core::convert::TryFrom<RuntimeSubcommand #ty_generics_with_inner> for RuntimeMessage #ty_generics_for_dest #where_clause_with_inner_clap_and_try_from {
                type Error = <__Dest as ::core::convert::TryFrom<__Inner>>::Error;
                /// Convert a `RuntimeSubcommand` to a `RuntimeSubcommand` with a different `__Inner` type using `try_from`.
                ///
                /// This method is called `try_map` instead of `try_from` to avoid conflicting with the `TryFrom` trait in
                /// the corner case where the source and destination types are the same.
                fn try_from(item: RuntimeSubcommand #ty_generics_with_inner ) -> Result<Self, Self::Error>
                 {
                    Ok(match item {
                        #( #try_from_subcommand_match_arms )*
                        RuntimeSubcommand::____phantom(_) => unreachable!(),
                    })
                }
            }

            impl #impl_generics ::sov_modules_api::CliWallet for #ident #ty_generics #where_clause_with_deserialize_bounds {
                type CliStringRepr<__Inner> = RuntimeMessage #ty_generics_with_inner;
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
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (named_type_defn, conversion_logic, subcommand_ident) = match &ast.data {
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
                /// An auto-generated version of the #ident_name::CallMessage enum which is guaranteed to have
                /// no anonymous fields. This is necessary to enable `clap`'s automatic CLI parsing.
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
            (enum_defn, from_body, item_with_named_fields_ident)
        }
        Data::Struct(s) => {
            let item_as_subcommand_ident = format_ident!("{}Subcommand", item_name);
            let mut named_fields = StructFieldExtractor::get_or_generate_named_fields(&s.fields);
            named_fields
                .iter_mut()
                .for_each(|field| field.filter_attrs(|attr| attr.path.is_ident("doc")));
            let field_names = named_fields.iter().map(|f| &f.ident).collect::<Vec<_>>();
            let conversion_logic = match s.fields {
                Fields::Named(_) => quote! {{
                        let #item_as_subcommand_ident:: #item_name {
                            args: #item_with_named_fields_ident { #(#field_names),* }
                        } = item;
                        #item_name{#(#field_names),*}
                }},
                Fields::Unnamed(_) => {
                    quote! {
                        let #item_as_subcommand_ident:: #item_name {
                            args: #item_with_named_fields_ident { #(#field_names),* }
                        } = item;
                        #item_name(#(#field_names),*)
                    }
                }
                Fields::Unit => quote! { #item_name },
            };

            let struct_docs = ast.attrs.iter().filter(|attr| attr.path.is_ident("doc"));
            let struct_defn = quote! {

                // An auto-generated version of the #ident_name::CallMessage struct which is guaranteed to have
                // no anonymous fields. This is necessary to enable `clap`'s automatic CLI parsing.
                #( #struct_docs )*
                #[derive(::clap::Args)]
                pub struct #item_with_named_fields_ident #generics {
                    #(#named_fields),*
                }

                /// An auto-generated single-variant enum wrapping a #ident_name::CallMessage struct. This enum
                /// implements `clap::Subcommand`, which simplifies code generation for the CLI parser.
                #[derive(::clap::Parser)]
                pub enum #item_as_subcommand_ident #generics {
                    #[command(arg_required_else_help(true))]
                    #item_name {
                        #[clap(flatten)]
                        args: #item_with_named_fields_ident #ty_generics
                    }
                }

            };
            (struct_defn, conversion_logic, item_as_subcommand_ident)
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                ast,
                "Unions are not supported as CLI wallet args",
            ))
        }
    };

    let expanded = quote! {
        // Create a new data type which matches the original, but with all fields named.
        // This is the type that Clap will parse the CLI args into
        #named_type_defn

        // Define a `From` implementation which converts from the named fields version to the original version
        impl #impl_generics From<#subcommand_ident #ty_generics> for #item_name #ty_generics #where_clause {
            fn from(item: #subcommand_ident #ty_generics) -> Self {
                #conversion_logic
            }
        }

        // Implement the `CliWalletArg` trait for the original type. This is what allows the original type to be used as a CLI arg.
        impl #impl_generics sov_modules_api::CliWalletArg for #item_name #ty_generics #where_clause {
            type CliStringRepr = #subcommand_ident #ty_generics;
        }
    };
    Ok(expanded.into())
}
