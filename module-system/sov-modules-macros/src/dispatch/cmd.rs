use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, Path, PathArguments,
    PathSegment, Type,
};

pub(crate) fn build_cmd_parser(
    input: DeriveInput,
    context_type: Type,
) -> Result<proc_macro::TokenStream, syn::Error> {
    let DeriveInput {
        attrs,
        vis,
        ident,
        generics,
        data,
    } = input.clone(); // make a copy to use later

    let data = match data {
        Data::Struct(data) => data,
        _ => return Err(syn::Error::new_spanned(ident, "expected a struct")),
    };

    let fields = match data.fields {
        Fields::Named(fields) => fields,
        _ => return Err(syn::Error::new_spanned(ident, "expected named fields")),
    };

    let match_arms: Vec<_> = fields
        .clone()
        .named
        .into_iter()
        .map(|field| {
            let field_name = field.ident.clone().unwrap();
            let field_name_string = field_name.to_string();
            let field_name_pascal_case = field_name_string.to_uppercase();
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
                    if let Some(PathSegment { arguments, .. }) = segments.last_mut() {
                        if let PathArguments::AngleBracketed(_) = arguments {
                            // Replace with an empty AngleBracketedGenericArguments
                            *arguments = PathArguments::None;
                        }
                    }
                    Path { segments, ..type_path.path.clone() }
                },
                _ => return Err(syn::Error::new_spanned(field, "expected a type path")),
            };

            Ok(quote! {
                #field_name_pascal_case => Ok({
                    #ident::<#context_type>::#encode_function_name(
                        serde_json::from_str::<<#type_path<#context_type> as sov_modules_api::Module>::CallMessage>(&call_data)?
                    )
                }),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Create tokens for original struct fields
    let original_struct_fields: Vec<_> = fields
        .named
        .into_iter()
        .map(|field| {
            let field_name = field.ident.unwrap();
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
