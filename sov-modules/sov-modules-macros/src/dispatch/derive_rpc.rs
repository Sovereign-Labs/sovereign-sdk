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


fn find_working_set_argument(sig: &Signature) -> Option<usize> {
    for (idx, input) in sig.inputs.iter().enumerate(){
        if let FnArg::Typed(PatType { ty, .. }) = input {
            if let syn::Type::Reference(syn::TypeReference { elem, .. }) = *ty.clone() {
                if let syn::Type::Path(syn::TypePath { path, .. }) = elem.as_ref() {
                    if let Some(segment) = path.segments.last() {
                        // TODO: enforce that the working set has exactly one angle bracketed argument
                        if segment.ident == "WorkingSet" && !segment.arguments.is_empty() {
                            return Some(idx);
                        }
                    }
                }
            }
        } 
    }
    None
}

struct RpcImplBlock {
    pub(crate) type_name: Ident,
    pub(crate) methods: Vec<RpcEnabledMethod>,
    pub(crate) working_set_type: Option<Type>,
    pub(crate) generics: syn::Generics,

}

struct RpcEnabledMethod {
    pub(crate) method_name: Ident,
    pub(crate) method_signature: Signature,
    pub(crate) idx_of_working_set_arg: Option<usize>,
}


impl RpcImplBlock {


    /// Builds the trait `_RpcImpl` That will be implemented by the runtim
    fn build_rpc_impl_trait(&self) -> proc_macro2::TokenStream {
        let mut impl_trait_methods = vec![];
        let impl_trait_name = format_ident!("{}RpcImpl", self.type_name);
        for method in self.methods.iter() {
            let arg_values = method.method_signature.inputs.clone().into_iter().map(|item| {
                if let FnArg::Typed(PatType { pat, .. }) = item {
                    if let syn::Pat::Ident(syn::PatIdent { ident, .. }) = *pat {
                        return quote! { #ident }
                    }
                    unreachable!("Expected a pattern identifier")
                } else {
                    quote! { self, }
                }
            });
            
            let signature = &method.method_signature;
            let method_name = &method.method_name;

            let impl_trait_method = if let Some(idx) = method.idx_of_working_set_arg  {
                let pre_working_set_args = arg_values.clone().take(idx);
                let post_working_set_args = arg_values.clone().skip(idx + 1);
                quote!{
                    #signature {
                        Self::get_backing_impl(self).#method_name(#(#pre_working_set_args),* &mut Self::get_working_set(self), #(#post_working_set_args),* )
                    }
                }
            } else {
                 quote!{
                    #signature {
                        Self::get_backing_impl(self).#method_name(#(#arg_values),* )
                    }
                }
            };
            impl_trait_methods.push(impl_trait_method);
        }

        let type_name = &self.type_name;
        let generics = &self.generics;
        let generics_params = generics.params.iter().map(|param| {
            if let syn::GenericParam::Type(syn::TypeParam { ident, .. }) = param {
                return quote! { #ident }
            }
            unreachable!("Expected a type parameter")
        }).collect::<Vec<_>>();

        if let Some(ref working_set_type) = self.working_set_type {
            quote! {
                pub trait #impl_trait_name #generics {
                    fn get_backing_impl(&self) -> & #type_name < #(#generics_params)*, >;
                    // TODO: Extract this method into a trait
                    fn get_working_set(&self) -> #working_set_type;
    
                    #(#impl_trait_methods)*
                }
            }
        } else {
            quote! {
                pub trait #impl_trait_name #generics {
                    fn get_backing_impl(&self) -> & #type_name < #(#generics_params)*, >;
    
                    #(#impl_trait_methods)*
                }
            }
        }
    }

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

    let generics = &input.generics;
    let mut rpc_info = RpcImplBlock {
        type_name,
        methods: vec![],
        working_set_type: None,
        generics: generics.clone(),
    };

    let mut intermediate_trait_items = vec![];
    let mut simplified_impl_items = vec![];
    for item in input.items.into_iter() {
        if let ImplItem::Method(ref method) = item {
            if let Some((attr, idx_of_rpc_attr)) = get_method_attribute(&method.attrs) {
                let idx_of_working_set_arg = find_working_set_argument(&method.sig);
                if let Some(idx) = idx_of_working_set_arg {
                    let arg = method.sig.inputs.iter().skip(idx).next().expect("WorkingSet arg was just verified to be present");
                    if let FnArg::Typed(PatType { ty, .. }) = arg {
                       rpc_info.working_set_type = Some(*ty.clone());
                    }
                }
                rpc_info.methods.push(RpcEnabledMethod{
                    method_name: method.sig.ident.clone(),
                    method_signature: method.sig.clone(),
                    idx_of_working_set_arg
                });
                let mut intermediate_signature = method.sig.clone();
                
                let annotated_signature = quote! {
                    #attr
                    #intermediate_signature;
                };
                intermediate_trait_items.push(annotated_signature);
                
                let mut original_method = method.clone();
                original_method.attrs.remove(idx_of_rpc_attr);
                simplified_impl_items.push(ImplItem::Method(original_method));
                continue
            } 
        }
        simplified_impl_items.push(item)
    }

    let impl_rpc_trait_impl = rpc_info.build_rpc_impl_trait();

    // Replace the original impl block with a new version with the rpc_gen and related annotations removed
    input.items = simplified_impl_items;
    let simplified_impl = quote! {
        #input
    };
   
    let rpc_output = quote! {
        #simplified_impl

        #impl_rpc_trait_impl

        #rpc_attribute
        pub trait #intermediate_trait_name  #generics {

            #(#intermediate_trait_items)*

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
