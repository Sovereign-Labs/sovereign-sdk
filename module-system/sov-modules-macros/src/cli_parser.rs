use proc_macro2::Span;
use crate::common::StructFieldExtractor;
use quote::{format_ident, quote};
use syn::{DeriveInput, Path, PathArguments, Type, Data, DataEnum, Fields, Ident, Meta, Lit, NestedMeta, AttributeArgs, MetaNameValue};
use syn::spanned::Spanned;

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
        context_type: Type,
    ) -> Result<proc_macro::TokenStream, syn::Error> {
        let DeriveInput {
            attrs,
            vis,
            ident,
            generics,
            data,
        } = input.clone();
        let fields = self.field_extractor.get_fields_from_struct(&data)?;

        let mut command_types = vec![];
        let mut module_commands = vec![];
        let mut module_args = vec![];
        let mut match_arms = vec![];

        // Loop over the fields
        for field in &fields {
            if field.attrs.iter().any(|attr| attr.path.is_ident("cli_skip")) {
                continue;
            }
            match &field.ty {
                syn::Type::Path(type_path) => {
                    let mut module_path = type_path.path.clone();
                    if let Some(segment) = module_path.segments.last_mut() {
                        if let syn::PathArguments::AngleBracketed(angle_bracketed_data) = &mut segment.arguments {
                            let mut new_args = Vec::new();
                            for gen_arg in &angle_bracketed_data.args {
                                match gen_arg {
                                    syn::GenericArgument::Type(syn::Type::Path(type_path)) if type_path.path.is_ident("C") => {
                                        new_args.push(syn::GenericArgument::Type(context_type.clone()));
                                    }
                                    _ => new_args.push(gen_arg.clone()),
                                }
                            }
                            angle_bracketed_data.args = new_args.into_iter().collect();
                        }

                        let module = segment.ident.clone();
                        let command_type_ident = format_ident!("{}Commands", module);
                        command_types.push(quote! {
                        type #command_type_ident = <#module_path as sov_modules_api::AutoClap>::ClapType;
                    });

                        let module_ident = format_ident!("{}", module);
                        let module_args_ident = format_ident!("{}Args", module);
                        module_commands.push(quote! {
                        #module_ident(#module_args_ident)
                    });

                        module_args.push(quote! {
                        #[derive(Parser)]
                        pub struct #module_args_ident {
                            #[clap(subcommand)]
                            /// Commands under #module
                            command: #command_type_ident,
                        }
                    });

                        let field_name = field.ident.clone();
                        let field_name_string = field_name.to_string();
                        let encode_function_name = format_ident!("encode_{}_call", field_name_string);

                        let type_path = match &field.ty {
                            Type::Path(type_path) => {
                                let mut segments = type_path.path.segments.clone();
                                let last = segments.last_mut().expect("Impossible happened! A type path has no segments");
                                last.arguments = PathArguments::None;
                                Path { segments, ..type_path.path.clone() }
                            },
                            _ => return Err(syn::Error::new_spanned(field.ident.clone(), "expected a type path")),
                        };

                        let type_name_string = type_path.segments.last().unwrap().ident.to_string();

                        match_arms.push(quote! {
                        #type_name_string => Ok({
                            #ident::<#context_type>::#encode_function_name(
                                serde_json::from_str::<<#type_path<#context_type> as sov_modules_api::Module>::CallMessage>(&call_data)?
                            )
                        }),
                    });
                    }
                },
                _ => {},
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

        // Merge and generate the new code
        let expanded = quote! {
        // re-declare the original struct
        #(#attrs)*
        #vis struct #ident #generics {
            #(#original_struct_fields),*
        }

        // generate the rest of the code
        #( #command_types )*
        /// List of utility commands
        #[derive(Parser)]
        pub enum ModuleCommands {
            #( #module_commands, )*
        }
        #( #module_args )*


        pub fn cmd_parser(module_name: &str, call_data: &str) -> anyhow::Result<Vec<u8>> {
            match module_name {
                #(#match_arms)*
                _ => panic!("unknown module name"),
            }
        }
    };
        Ok(expanded.into())
    }
}

// pub fn derive_clap_custom_enum(mut ast: DeriveInput) -> Result<proc_macro::TokenStream, syn::Error> {
//     let enum_name = &ast.ident;
//     let generics = &ast.generics;
//
//     // Extract module_name attribute
//     let module_name = ast
//         .attrs
//         .iter()
//         .find_map(|attr| attr.parse_meta().ok().and_then(|meta| {
//             if meta.path().is_ident("module_name") {
//                 match meta {
//                     Meta::NameValue(MetaNameValue { lit: Lit::Str(lit_str), .. }) => Some(lit_str),
//                     _ => None,
//                 }
//             } else {
//                 None
//             }
//         }))
//         .ok_or_else(|| syn::Error::new(enum_name.span(), "Expected module_name attribute"))?;
//
//     let trait_type_ident = syn::Ident::new(&module_name.value(), module_name.span());
//
//     if let Data::Enum(DataEnum { variants, .. }) = ast.data.clone() {
//         let mut variants_with_fields = vec![];
//         let mut convert_cases = vec![];
//
//         let cli_enum_with_fields_ident = Ident::new(
//             &format!("{}WithNamedFields", enum_name),
//             proc_macro2::Span::call_site(),
//         );
//
//         for variant in variants {
//             let ident = &variant.ident;
//             let doc_attrs_variant = variant.attrs.iter()
//                 .filter(|attr| attr.path.is_ident("doc"))
//                 .collect::<Vec<_>>();
//
//             match &variant.fields {
//                 Fields::Unnamed(unnamed_fields) => {
//                     let named_fields = unnamed_fields.unnamed.iter().enumerate().map(|(i, field)| {
//                         let name = Ident::new(&format!("field{}", i), field.span());
//                         let ty = &field.ty;
//                         let doc_attrs_field = field.attrs.iter()
//                             .filter(|attr| attr.path.is_ident("doc"))
//                             .collect::<Vec<_>>();
//                         quote! {
//                             #( #doc_attrs_field )*
//                             #name: #ty
//                         }
//                     }).collect::<Vec<_>>();
//                     variants_with_fields.push(quote! {
//                         #( #doc_attrs_variant )*
//                         #ident {#(#named_fields),*}
//                     });
//
//                     let fields = unnamed_fields.unnamed.iter().enumerate().map(|(i, _)| {
//                         let name = Ident::new(&format!("field{}", i), unnamed_fields.span());
//                         quote! {#name}
//                     }).collect::<Vec<_>>();
//
//                     convert_cases.push(quote! {
//                         #cli_enum_with_fields_ident::#ident {#(#fields),*} => #enum_name::#ident(#(#fields),*),
//                     });
//                 },
//                 Fields::Named(fields_named) => {
//                     let field_tokens = fields_named.named.iter().map(|field| {
//                         let name = field.ident.as_ref().unwrap();
//                         let ty = &field.ty;
//                         let doc_attrs_field = field.attrs.iter()
//                             .filter(|attr| attr.path.is_ident("doc"))
//                             .collect::<Vec<_>>();
//                         quote! {
//                             #( #doc_attrs_field )*
//                             #name: #ty
//                         }
//                     }).collect::<Vec<_>>();
//
//                     variants_with_fields.push(quote! {
//                         #( #doc_attrs_variant )*
//                         #ident {#(#field_tokens),*}
//                     });
//
//                     let fields = fields_named.named.iter().map(|field| {
//                         let name = field.ident.as_ref().unwrap();
//                         quote! {#name}
//                     }).collect::<Vec<_>>();
//
//                     convert_cases.push(quote! {
//                         #cli_enum_with_fields_ident::#ident {#(#fields),*} => #enum_name::#ident {#(#fields),*},
//                     });
//                 },
//                 Fields::Unit => {
//                     variants_with_fields.push(quote! {
//                         #( #doc_attrs_variant )*
//                         #ident
//                     });
//                     convert_cases.push(quote! {
//                         #cli_enum_with_fields_ident::#ident => #enum_name::#ident,
//                     });
//                 }
//             }
//         }
//
//         let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
//         let expanded = quote! {
//             #[derive(clap::Parser)]
//             pub enum #cli_enum_with_fields_ident #impl_generics #where_clause {
//                 #(#variants_with_fields,)*
//             }
//
//             impl #impl_generics From<#cli_enum_with_fields_ident #ty_generics> for #enum_name #ty_generics #where_clause {
//                 fn from(item: #cli_enum_with_fields_ident #ty_generics) -> Self {
//                     match item {
//                         #(#convert_cases)*
//                     }
//                 }
//             }
//
//             impl<C: sov_modules_api::Context> sov_modules_api::AutoClap for #trait_type_ident<C> {
//                 type ClapType = #cli_enum_with_fields_ident<C>;
//             }
//         };
//
//         Ok(expanded.into())
//     } else {
//         panic!("This derive macro only works with enums");
//     }
// }


pub fn derive_clap_custom_enum(mut ast: DeriveInput) -> Result<proc_macro::TokenStream, syn::Error> {
    let enum_name = &ast.ident;
    let generics = &ast.generics;

    // Extract module_name attribute
    let module_name = ast
        .attrs
        .iter()
        .find_map(|attr| attr.parse_meta().ok().and_then(|meta| {
            if meta.path().is_ident("module_name") {
                match meta {
                    Meta::NameValue(MetaNameValue { lit: Lit::Str(lit_str), .. }) => Some(lit_str),
                    _ => None,
                }
            } else {
                None
            }
        }))
        .ok_or_else(|| syn::Error::new(enum_name.span(), "Expected module_name attribute"))?;

    let trait_type_ident = syn::Ident::new(&module_name.value(), module_name.span());

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
            let doc_attrs_variant = variant.attrs.iter()
                .filter(|attr| attr.path.is_ident("doc"))
                .collect::<Vec<_>>();

            match &variant.fields {
                Fields::Unnamed(unnamed_fields) => {
                    let named_fields = unnamed_fields.unnamed.iter().enumerate().map(|(i, field)| {
                        let name = Ident::new(&format!("field{}", i), field.span());
                        let ty = &field.ty;
                        let doc_attrs_field = field.attrs.iter()
                            .filter(|attr| attr.path.is_ident("doc"))
                            .collect::<Vec<_>>();
                        quote! {
                            #( #doc_attrs_field )*
                            #name: #ty
                        }
                    }).collect::<Vec<_>>();
                    variants_with_fields.push(quote! {
                        #( #doc_attrs_variant )*
                        #ident {#(#named_fields),*}
                    });

                    let fields = unnamed_fields.unnamed.iter().enumerate().map(|(i, _)| {
                        let name = Ident::new(&format!("field{}", i), unnamed_fields.span());
                        quote! {#name}
                    }).collect::<Vec<_>>();

                    convert_cases.push(quote! {
                        #cli_enum_with_fields_ident::#ident {#(#fields),*} => #enum_name::#ident(#(#fields),*),
                    });
                },
                Fields::Named(fields_named) => {
                    let field_tokens = fields_named.named.iter().map(|field| {
                        let name = field.ident.as_ref().unwrap();
                        let ty = &field.ty;
                        let doc_attrs_field = field.attrs.iter()
                            .filter(|attr| attr.path.is_ident("doc"))
                            .collect::<Vec<_>>();
                        quote! {
                            #( #doc_attrs_field )*
                            #name: #ty
                        }
                    }).collect::<Vec<_>>();

                    variants_with_fields.push(quote! {
                        #( #doc_attrs_variant )*
                        #ident {#(#field_tokens),*}
                    });

                    let fields = fields_named.named.iter().map(|field| {
                        let name = field.ident.as_ref().unwrap();
                        quote! {#name}
                    }).collect::<Vec<_>>();

                    convert_cases.push(quote! {
                        #cli_enum_with_fields_ident::#ident {#(#fields),*} => #enum_name::#ident {#(#fields),*},
                    });
                },
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
            quote! { #cli_enum_with_fields_ident<C> }
        } else {
            quote! { #cli_enum_with_fields_ident }
        };

        let expanded = quote! {
            #[derive(clap::Parser)]
            pub enum #cli_enum_with_fields_ident #impl_generics #where_clause {
                #(#variants_with_fields,)*
            }

            impl #impl_generics From<#cli_enum_with_fields_ident #ty_generics> for #enum_name #ty_generics #where_clause {
                fn from(item: #cli_enum_with_fields_ident #ty_generics) -> Self {
                    match item {
                        #(#convert_cases)*
                    }
                }
            }

            impl<C: sov_modules_api::Context> sov_modules_api::AutoClap for #trait_type_ident<C> {
                type ClapType = #clap_type;
            }
        };

        Ok(expanded.into())
    } else {
        panic!("This derive macro only works with enums");
    }
}
