use quote::{format_ident, quote};
use syn::token::Colon;
use syn::DeriveInput;

use crate::common::{pascal_case_ident, StructFieldExtractor};

pub(crate) fn derive_cli_wallet(
    name: &'static str,
    input: DeriveInput,
) -> syn::Result<proc_macro::TokenStream> {
    let field_extractor = StructFieldExtractor::new(name);
    let DeriveInput {
        ident,
        generics,
        data,
        ..
    } = input;
    let fields = field_extractor.get_fields_from_struct(&data)?;

    let (_, ty_generics, _) = generics.split_for_impl();

    let mut module_json_parser_arms = vec![];
    let mut module_message_arms = vec![];
    let mut tx_args_subcommand_match_arms_chain_id = vec![];
    let mut tx_args_subcommand_match_arms_max_priority_fee_bips = vec![];
    let mut tx_args_subcommand_match_arms_max_fee = vec![];
    let mut tx_args_subcommand_match_arms_gas_limit = vec![];
    let mut try_from_subcommand_match_arms = vec![];
    let mut try_map_match_arms = vec![];
    let mut from_json_match_arms = vec![];
    let mut deserialize_constraints: Vec<syn::WherePredicate> = vec![];

    // Loop over the fields.
    for field in &fields {
        // Skip fields with the attribute `cli_skip`.
        if field.contains_attr("cli_skip") {
            continue;
        }

        // For each type path we encounter, we need to extract the generic type parameters for that field
        // and construct a `Generics` struct that contains the bounds for each of those generic type parameters.
        let syn::Type::Path(type_path) = &field.ty else {
            continue;
        };

        let module_path = type_path.path.clone();
        let field_name = pascal_case_ident(&field.ident);
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
                    ::sov_modules_api::prelude::serde_json::from_str::<
                        <#module_path as ::sov_modules_api::Module>::CallMessage
                    >(&contents.json).map(
                        // Use the enum variant as a constructor
                        <#ident #ty_generics as ::sov_modules_api::DispatchCall>::Decodable:: #field_name
                    )
                },
            });

        try_map_match_arms.push(quote! {
            RuntimeMessage::#field_name { contents } => RuntimeMessage::#field_name { contents: contents.try_into()? },
        });

        tx_args_subcommand_match_arms_chain_id.push(quote! {
            RuntimeSubcommand::#field_name { contents } => <__Inner as ::sov_modules_api::cli::CliTxImportArg>::chain_id(&contents),
        });

        tx_args_subcommand_match_arms_max_priority_fee_bips.push(quote! {
            RuntimeSubcommand::#field_name { contents } => <__Inner as ::sov_modules_api::cli::CliTxImportArg>::max_priority_fee_bips(&contents),
        });

        tx_args_subcommand_match_arms_max_fee.push(quote! {
            RuntimeSubcommand::#field_name { contents } => <__Inner as ::sov_modules_api::cli::CliTxImportArg>::max_fee(&contents),
        });

        tx_args_subcommand_match_arms_gas_limit.push(quote! {
            RuntimeSubcommand::#field_name { contents } => <__Inner as ::sov_modules_api::cli::CliTxImportArg>::gas_limit(&contents),
        });

        try_from_subcommand_match_arms.push(quote! {
            RuntimeSubcommand::#field_name { contents } => RuntimeMessage::#field_name { contents: contents.try_into()? },
        });

        // Build a constraint requiring that all call messages support serde deserialization
        let deserialization_constraint = {
            let type_path: syn::TypePath =
                syn::parse_quote! {<#module_path as ::sov_modules_api::Module>::CallMessage };
            let bounds: syn::TypeParamBound = syn::parse_quote! {::serde::de::DeserializeOwned};
            syn::WherePredicate::Type(syn::PredicateType {
                lifetimes: None,
                bounded_ty: syn::Type::Path(type_path),
                colon_token: Colon::default(),
                bounds: vec![bounds].into_iter().collect(),
            })
        };
        deserialize_constraints.push(deserialization_constraint);
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
                    .push(syn::parse_quote! { __Inner: clap::Args });
                Some(result)
            }
            None => syn::parse_quote! {
                where  __Inner: clap::Args
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
                .push(syn::parse_quote! { __Dest: ::core::convert::TryFrom<__Inner> });
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

    let inner_module_ident = format_ident!("__{}_cli_parser_inner_module", ident);

    // Merge and generate the new code
    let expanded = quote! {

        #[doc(inline)]
        pub use #inner_module_ident::*;

        #[allow(non_snake_case)]
        mod #inner_module_ident {
            use super::*;
            use ::sov_modules_api::prelude::clap;

            /// An enum expressing the subcommands available to this runtime. Contains
            /// one subcommand for each module, except modules annotated with the #[cli_skip] attribute
            #[derive(clap::Parser)]
            #[allow(non_camel_case_types)]
            pub enum RuntimeSubcommand #impl_generics_with_inner #where_clause_with_inner_as_clap {
                #( #module_json_parser_arms, )*
                #[clap(skip)]
                #[doc(hidden)]
                ____phantom(::std::marker::PhantomData<#ident #ty_generics>)
            }

            impl #impl_generics_with_inner ::sov_modules_api::cli::CliTxImportArg for RuntimeSubcommand #ty_generics_with_inner #where_clause_with_deserialize_bounds, __Inner: clap::Args + ::sov_modules_api::cli::CliTxImportArg {
                fn chain_id(&self) -> u64 {
                    match self {
                        #( #tx_args_subcommand_match_arms_chain_id )*
                        _ => unreachable!(),
                    }
                }

                fn max_priority_fee_bips(&self) -> u64 {
                    match self {
                        #( #tx_args_subcommand_match_arms_max_priority_fee_bips )*
                        _ => unreachable!(),
                    }
                }

                fn max_fee(&self) -> ::sov_modules_api::Amount {
                    match self {
                        #( #tx_args_subcommand_match_arms_max_fee )*
                        _ => unreachable!(),
                    }
                }

                fn gas_limit(&self) -> Option<&[u64]> {
                    match self {
                        #( #tx_args_subcommand_match_arms_gas_limit )*
                        _ => unreachable!(),
                    }
                }
            }

            impl #impl_generics_with_inner ::sov_modules_api::cli::CliFrontEnd<#ident #ty_generics> for RuntimeSubcommand #ty_generics_with_inner #where_clause_with_deserialize_bounds, __Inner: clap::Args {
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
                type Error = ::sov_modules_api::prelude::serde_json::Error;
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
        }
    };
    Ok(expanded.into())
}
