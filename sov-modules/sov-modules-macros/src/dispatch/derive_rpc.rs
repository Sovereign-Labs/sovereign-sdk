use proc_macro2::Ident;
use quote::{format_ident, quote, ToTokens};
use syn::{Attribute, FnArg, ImplItem, Meta, MetaList, Path, PathSegment, Type, Signature, PatType};

/// Retrns an attribute with the name `rpc_method` replaced with `method`, and the index
/// into the argument array where the attribute was found.
fn get_method_attribute(attributes: &[Attribute]) -> Option<(Attribute, usize)> {
    for (idx, attribute) in attributes.iter().enumerate() {
        if let Ok(Meta::List(MetaList { path, .. })) = attribute.parse_meta() {
            if path.is_ident("rpc_method") {
                let mut new_attr = attribute.clone();
                let path = &mut new_attr.path;
                path.segments.last_mut().unwrap().ident = format_ident!("method");
                return Some((new_attr, idx));
            }
        }
    }
    None
}

/// A handy function that gpt4 generated to convert snake-case identifiers to camel-case
fn intermediate_trait_name(ident: &Ident) -> Ident {
    let mut ident_str = ident.to_string();
    ident_str.push_str("Rpc");

    format_ident!("{}", ident_str)
}

// pub(crate) fn derive_rpc(
//     mut attrs: Vec<syn::NestedMeta>,
//     input: syn::ItemImpl,
// ) -> Result<proc_macro2::TokenStream, syn::Error> {
//     let generics = &input.generics;
//     let type_name = match *input.self_ty {
//         syn::Type::Path(ref type_path) => &type_path.path.segments.last().unwrap().ident,
//         _ => return Err(syn::Error::new_spanned(input.self_ty, "Invalid type")),
//     };

    // for attr in attrs.iter_mut() {
    //     match attr {
    //         syn::NestedMeta::Meta(syn::Meta::Path(path)) => {
    //             if path.is_ident("rpc_gen") {
    //                 path.segments.last_mut().unwrap().ident = format_ident!("rpc");
    //             }
    //         }
    //         _ => {}
    //     }
    // }

//     let intermediate_trait_name = intermediate_trait_name(type_name);

//     let mut methods = vec![];
//     for item in input.items.iter() {
//         if let ImplItem::Method(method) = item {
//             if let Some(attr) = get_method_attribute(&method.attrs) {
//                 let signature = method.sig.to_token_stream();
//                 // methods.push(signature)
//                 let annotated_signature = quote! {
//                     #attr
//                     #signature
//                 };
//                 methods.push(annotated_signature)
//             }
//         }
//     }

//     let attrs: Vec<proc_macro2::TokenStream> =
//         attrs.into_iter().map(|a| a.to_token_stream()).collect();

//     let intermediate_trait = quote! {
//         #[jsonrpsee::async_trait]
//         #(#attrs)*
//         impl #generics #intermediate_trait_name for #type_name #generics #input.where_clause {
//             #(#methods)*

//             #[method(name = "health")]
//             fn health() -> Result<(), jsonrpsee::Error> {
//                 Ok(())
//             }

//         }
//     };

//     Ok(intermediate_trait)
// }

fn jsonrpsee_rpc_macro_path() -> Path {
    let segments = vec![
        Ident::new("jsonrpsee", proc_macro2::Span::call_site()),
        Ident::new("proc_macros", proc_macro2::Span::call_site()),
        Ident::new("rpc", proc_macro2::Span::call_site()),
    ];

    let path_segments = segments
        .into_iter()
        .map(|ident| PathSegment {
            ident,
            arguments: syn::PathArguments::None,
        });

        Path {
        leading_colon: Some(syn::Token![::](proc_macro2::Span::call_site())),
        segments: syn::punctuated::Punctuated::from_iter(path_segments),
    }
}

// fn nested_meta_to_attribute(nested_meta: Vec<syn::NestedMeta>) -> Attribute {
//     let path = jsonrpsee_rpc_macro_path();
//     let meta = Meta::List(MetaList {
//         path,
//         paren_token: syn::token::Paren { span: proc_macro2::Span::call_site() },
//         nested: syn::punctuated::Punctuated::from_iter(nested_meta.into_iter()),
//     });

//     Attribute {
//         pound_token: syn::token::Pound { spans: [proc_macro2::Span::call_site()] },
//         style: syn::AttrStyle::Outer,
//         bracket_token: syn::token::Bracket { span: proc_macro2::Span::call_site() },
//         path: meta.path().clone(),
//         tokens: meta.to_token_stream(),
//     }
// }


fn remove_working_set_arguments(sig: &mut Signature) -> Option<usize> {
    let target_type: syn::Type = syn::parse_quote! { &mut ::sov_modules_api::WorkingSet };
    let mut idx_of_working_set_arg = None;
    sig.inputs = sig.inputs.clone().into_iter().enumerate().filter(|(idx, input)| {
        if let FnArg::Typed(PatType { ty, .. }) = input {
            if let syn::Type::Reference(syn::TypeReference { elem, .. }) = *ty.clone() {
                if let syn::Type::Path(syn::TypePath { path, .. }) = elem.as_ref() {
                    if let Some(segment) = path.segments.last() {
                        // TODO: enforce that the working set has exactly one angle bracketed argument
                        if segment.ident == "WorkingSet" && !segment.arguments.is_empty() {
                            idx_of_working_set_arg = Some(*idx);
                            return false
                        }
                    }
                }
            }
            true
        } else {
            true
        }
    }).map(|(idx, item)| item).collect();

    return idx_of_working_set_arg
}

fn build_rpc_trait(attrs: &proc_macro2::TokenStream, type_name: Ident, mut input: syn::ItemImpl) -> Result<proc_macro2::TokenStream, syn::Error> {
    let intermediate_trait_name = format_ident!("{}Rpc", type_name);

    let wrapped_attr_args = quote! {
        (#attrs)
    };


    let rpc_attribute = syn::Attribute {
        pound_token: syn::token::Pound { spans: [proc_macro2::Span::call_site()] },
        style: syn::AttrStyle::Outer,
        bracket_token: syn::token::Bracket { span: proc_macro2::Span::call_site() },
        path: jsonrpsee_rpc_macro_path(),
        tokens: wrapped_attr_args,
    };


    // Iterate over the methods from the `impl` block, building up three lists of items as we go

    // 1. The methods from the original `impl` block.
    let mut impl_items = vec![];
    // 2. The methods that will be added to the intermediate trait which is consumed by #[jsonrpsee::rpc]
    let mut intermediate_trait_methods = vec![];
    // 3. The methods that are for the _RpcImpl trait, which will be implemented by the runtime
    let mut impl_trait_methods = vec![];
    for item in input.items.into_iter() {
        if let ImplItem::Method(ref method) = item {
            if let Some((attr, idx_of_rpc_attr)) = get_method_attribute(&method.attrs) {
                let mut intermediate_signature = method.sig.clone();
                let idx_of_working_set_arg = remove_working_set_arguments(&mut intermediate_signature);
                let annotated_signature = quote! {
                    #attr
                    #intermediate_signature;
                };
                intermediate_trait_methods.push(annotated_signature);
                let mut impl_method = method.clone();
                impl_method.attrs.remove(idx_of_rpc_attr);

                let method_name = method.sig.ident.clone();

                let arg_values = method.sig.inputs.clone().into_iter().map(|item| {
                    if let FnArg::Typed(PatType { pat, .. }) = item {
                        if let syn::Pat::Ident(syn::PatIdent { ident, .. }) = *pat {
                            return quote! { #ident }
                        }
                        unreachable!("Expected a pattern identifier")
                    } else {
                        quote! { self, }
                    }
                });
                let impl_trait_method = if let Some(idx) = idx_of_working_set_arg  {
                    let pre_working_set_args = arg_values.clone().take(idx);
                    let post_working_set_args = arg_values.clone().skip(idx + 1);
                    quote!{
                        #intermediate_signature {
                            Self::get_backing_impl(self).#method_name(#(#pre_working_set_args),* &mut Self::get_working_set(self), #(#post_working_set_args),* )
                        }
                    }
                } else {
                     quote!{
                        #intermediate_signature {
                            Self::get_backing_impl(self).#method_name(#(#arg_values),* )
                        }
                    }
                };
                // println!("impl_trait_method: {}", impl_trait_method.to_string());
                impl_trait_methods.push(impl_trait_method);

                impl_items.push(ImplItem::Method(impl_method));
                continue
            } 
        }
        impl_items.push(item)
    }

    input.items = impl_items;

    let reduced_impl = quote! {
        #input
    };

    let impl_trait_name = format_ident!("{}RpcImpl", type_name);


    let generics = &input.generics;
    let generics_params = &input.generics.params.iter().map(|param| {
        if let syn::GenericParam::Type(syn::TypeParam { ident, .. }) = param {
            return quote! { #ident }
        }
        unreachable!("Expected a type parameter")
    }).collect::<Vec<_>>();
    let generics_where_clause = &input.generics.where_clause;

    // TODO: Get robust detection of the working set type
    let impl_trait_mpl = quote! {
        pub trait #impl_trait_name #generics  {
            fn get_backing_impl(&self) -> & #type_name < #(#generics_params)*, >;
            // TODO: Extract this method into a trait
            fn get_working_set(&self) -> ::sov_modules_api::WorkingSet<C>;

            #(#impl_trait_methods)*
        }
    };


    println!("impl_trait_impl: {}", impl_trait_mpl.to_string());
   
    let rpc_output = quote! {
        #reduced_impl

        #impl_trait_mpl

        #rpc_attribute
        pub trait #intermediate_trait_name {

            #(#intermediate_trait_methods)*

            #[method(name = "health")]
            fn health(&self) -> ::jsonrpsee::core::RpcResult<()> {
                Ok(())
            }

        }
    };


    Ok(rpc_output)
}


pub(crate) fn derive_rpc(
    attrs: proc_macro2::TokenStream,
    input: syn::ItemImpl,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let generics = &input.generics;

    let type_name = match *input.self_ty {
        syn::Type::Path(ref type_path) => &type_path.path.segments.last().unwrap().ident,
        _ => return Err(syn::Error::new_spanned(input.self_ty, "Invalid type")),
    };

    build_rpc_trait(&attrs, type_name.clone(), input)

}
