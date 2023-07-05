use quote::{format_ident, quote};
use syn::{DeriveInput, Path, PathArguments, Type};

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

    pub(crate) fn cli_parser(
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
        } = input;

        let fields = self.field_extractor.get_fields_from_struct(&data)?;

        let match_arms: Vec<_> = fields
            .clone()
            .into_iter()
            .map(|field| {
                let field_name = field.ident.clone();
                let field_name_string = field_name.to_string();
                let encode_function_name = format_ident!("encode_{}_call", field_name_string);

                // TODO:
                // For the initial version, before complicating the macro,
                // we're assuming that each module type in Runtime only has
                // one generic. we're removing that and appending the concrete
                // that's passed in from the macro.
                // We need to fix this so that:
                // 1. Determine which generic has the Context bound
                // 2. Identify only that generic from the module type and replace it
                // 3. Retain other generics

                // Extract the type name
                let type_path = match &field.ty {
                    Type::Path(type_path) => {
                        let mut segments = type_path.path.segments.clone();
                        let last = segments.last_mut().expect("Impossible happened! A type path has no segments");
                        last.arguments = PathArguments::None;
                        Path { segments, ..type_path.path.clone() }
                    },
                    _ => return Err(syn::Error::new_spanned(field.ident, "expected a type path")),
                };

                let type_name_string = type_path.segments.last().unwrap().ident.to_string();

                Ok(quote! {
                #type_name_string => Ok({
                    #ident::<#context_type>::#encode_function_name(
                        serde_json::from_str::<<#type_path<#context_type> as sov_modules_api::Module>::CallMessage>(&call_data)?
                    )
                }),
            })
            })
            .collect::<Result<Vec<_>, _>>()?;

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

        let cmd_parser_tokens = quote! {
            #(#attrs)*
            #vis struct #ident #generics {
                #(#original_struct_fields),*
            }

            pub fn cmd_parser(module_name: &str, call_data: &str) -> anyhow::Result<Vec<u8>> {
                match module_name {
                    #(#match_arms)*
                    _ => panic!("unknown module name"),
                }
            }
        };

        Ok(cmd_parser_tokens.into())
    }
}
