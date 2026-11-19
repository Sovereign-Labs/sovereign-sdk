use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{parse_quote, Attribute, FnArg, ImplItem, PatType, Signature};

use self::utils::*;
use crate::common::doc_attributes;

struct RpcMethod {
    method: syn::ImplItemFn,
    docs: Vec<Attribute>,
    rpc_attribute: RpcMethodAttribute,
    api_state_accessor_arg: Option<ApiStateAccessorArg>,
    extensions_arg: Option<ExtensionsArg>,
}

impl RpcMethod {
    fn parse(method: &syn::ImplItemFn) -> syn::Result<Self> {
        let mut rpc_attribute = RpcMethodAttribute::parse(method)?;
        let api_state_accessor_arg = ApiStateAccessorArg::parse(&method.sig)?;
        let extensions_arg = ExtensionsArg::parse(&method.sig)?;
        match (&extensions_arg, rpc_attribute.has_with_extensions()) {
            (Some(_), _) => rpc_attribute.ensure_with_extensions(),
            (None, true) => {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "`with_extensions` requires a `&jsonrpsee::Extensions` argument.",
                ));
            }
            (None, false) => {}
        }

        let docs = doc_attributes(&method.attrs);

        Ok(Self {
            method: method.clone(),
            rpc_attribute,
            docs,
            api_state_accessor_arg,
            extensions_arg,
        })
    }

    /// Returns an identical copy of the original method, but with the `#[method_rpc]`
    /// attribute removed.
    fn method_without_rpc_attr(&self) -> syn::ImplItemFn {
        let mut method = self.method.clone();
        method.attrs.remove(self.rpc_attribute.index_within_attrs);
        method
    }

    fn name(&self) -> &Ident {
        &self.method.sig.ident
    }

    fn signature(&self) -> &Signature {
        &self.method.sig
    }

    /// Builds the annotated signature for this method.
    fn annotated_signature_for_rpc_trait(&self) -> TokenStream {
        let mut method_signature = self.method.sig.clone();

        // Remove the working set and extensions arguments from the method signature, if present.
        // jsonrpsee adds the extensions argument back to the server trait by itself.
        let removed_idxs = [
            self.api_state_accessor_arg.as_ref().map(|arg| arg.idx),
            self.extensions_arg.as_ref().map(|arg| arg.idx),
        ];
        method_signature.inputs = method_signature
            .inputs
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !removed_idxs.contains(&Some(*i)))
            .map(|(_, arg)| arg)
            .collect();

        let docs = &self.docs;
        let rpc_attribute = &self.rpc_attribute.attr;

        quote! {
            #( #docs )*
            #rpc_attribute
            #method_signature;
        }
    }
}

struct RpcImplBlock {
    pub module_type: syn::Type,
    pub methods: Vec<RpcMethod>,
    pub api_state_accessor_type: Option<syn::Type>,
}

impl RpcImplBlock {
    fn rpc_server_trait_item(&self, method: &RpcMethod) -> syn::Result<TokenStream> {
        let docs = &method.docs;
        let arg_names = function_arg_names(method.signature())?;
        let method_name = &method.name();
        let module_type = &self.module_type;

        let state_idx = method.api_state_accessor_arg.as_ref().map(|arg| arg.idx);
        let ext_idx = method.extensions_arg.as_ref().map(|arg| arg.idx);

        let mut inputs: Vec<FnArg> = method
            .signature()
            .inputs
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != state_idx && Some(*i) != ext_idx) // Drop the state checkpoint and extensions arguments.
            .map(|(_, arg)| arg.clone())
            .collect();
        if let Some(idx) = ext_idx {
            // jsonrpsee's server trait expects the extensions argument right after `self`.
            inputs.insert(1.min(inputs.len()), method.signature().inputs[idx].clone());
        }
        let mut signature = method.signature().clone();
        signature.inputs = inputs.into_iter().collect();

        let build_api_state_accessor = method.api_state_accessor_arg.as_ref().map(|arg| {
            let api_state_accessor_ident = &arg.ident;
            quote! {
                let #api_state_accessor_ident = &mut Self::build_api_state_accessor(self, None).expect("Impossible to build a default api state accessor. This is a bug. Please report it.");
            }
        });

        Ok(quote! {
            #( #docs )*
            #signature {
                #build_api_state_accessor
                <#module_type>::#method_name(#(#arg_names),*)
            }
        })
    }

    /// If the state checkpoint type is not set, set it.
    /// If it is, we need to check that it's the same type.
    fn set_api_state_accessor_type(&mut self, method: &RpcMethod) -> syn::Result<()> {
        let method_checkpoint_type = method
            .api_state_accessor_arg
            .as_ref()
            .map(|arg| arg.ty.clone());
        match (&self.api_state_accessor_type, &method_checkpoint_type) {
            (Some(ws), Some(ref method_ws_type)) if ws != method_ws_type => {
                return Err(syn::Error::new_spanned(
                    method.name(),
                    format!("All `#[rpc_method]` annotated methods must have the same state checkpoint type. Found `{ws:?}` and `{method_ws_type:?}`"),
                ));
            }
            // The method has no working set argument; do nothing.
            (_, None) => {}
            _ => self.api_state_accessor_type = method_checkpoint_type,
        };

        Ok(())
    }
}

fn inner_rpc_gen(
    mut attr_contents: Vec<syn::Meta>,
    input: &syn::ItemImpl,
    type_name: &Ident,
) -> syn::Result<TokenStream> {
    // If the user hasn't directly provided trait bounds, override jsonrpsee's
    // defaults with an empty bound. This prevents spurious compilation errors
    // like `Spec does not implement DeserializeOwned`.
    add_attr_meta_if_missing(&mut attr_contents, syn::parse_quote! { client_bounds() });
    add_attr_meta_if_missing(&mut attr_contents, syn::parse_quote! { server_bounds() });
    // Iterate over the methods from the `impl` block, building up three lists of items as we go

    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let module_type = quote! { #type_name #ty_generics };

    let mut rpc_info = RpcImplBlock {
        methods: vec![],
        module_type: parse_quote! { #module_type },
        api_state_accessor_type: None,
    };

    let mut rpc_trait_items = vec![];
    let mut bare_impl_block_items = vec![];

    // Iterate over the methods from the `impl` block, building up lists of
    // items (trait method definitions and server method implementations) as we
    // go.
    for item in &input.items {
        if let ImplItem::Fn(ref method) = item {
            let method = RpcMethod::parse(method)?;

            rpc_info.set_api_state_accessor_type(&method)?;
            rpc_trait_items.push(method.annotated_signature_for_rpc_trait());
            bare_impl_block_items.push(ImplItem::Fn(method.method_without_rpc_attr()));

            rpc_info.methods.push(method);
        }
    }
    let rpc_server_trait_items = rpc_info
        .methods
        .iter()
        .map(|method| rpc_info.rpc_server_trait_item(method))
        .collect::<Result<Vec<_>, _>>()?;

    // Replace the original impl block with a new version with the rpc_gen and
    // related annotations removed.
    let mut bare_impl_block = input.clone();
    bare_impl_block.items = bare_impl_block_items;

    let rpc_trait_name = format_ident!("{}Rpc", type_name);
    let rpc_server_trait_name = format_ident!("{}RpcServer", type_name);
    let rpc_trait_attrs = {
        let doc_string = format!("Generated RPC trait for `{type_name}`.");
        quote! {
            #[doc = #doc_string]
            #[::jsonrpsee::proc_macros::rpc(#(#attr_contents,)*)]
        }
    };

    Ok(quote! {
        #bare_impl_block

        #rpc_trait_attrs
        pub trait #rpc_trait_name #generics #where_clause {
            #(#rpc_trait_items)*

            /// Check the health of the RPC server
            #[method(name = "health")]
            fn health(&self) -> ::jsonrpsee::core::RpcResult<()> {
                Ok(())
            }

            /// Get the ID of this module
            #[method(name = "moduleId")]
            fn module_id(&self) -> ::jsonrpsee::core::RpcResult<String> {
                let module = ::core::default::Default::default();
                Ok(<#module_type as ::sov_modules_api::ModuleInfo>::id(&module).to_string())
            }
        }

        impl #impl_generics #rpc_server_trait_name #ty_generics for sov_modules_api::rest::ApiState<<#module_type as ::sov_modules_api::ModuleInfo>::Spec, #module_type> #where_clause {
            #(#rpc_server_trait_items)*
        }

        impl #impl_generics sov_modules_api::__rpc_macros_private::ModuleWithRpcServer for #module_type #where_clause {
            type Spec = <Self as sov_modules_api::ModuleInfo>::Spec;

            fn rpc_methods(
                &self,
                api_state: sov_modules_api::rest::ApiState<Self::Spec>,
            ) -> jsonrpsee::RpcModule<()> {
                api_state.with::<Self>(self.clone()).into_rpc().remove_context()
            }
        }
    })
}

pub fn rpc_gen(attr_contents: Vec<syn::Meta>, input: syn::ItemImpl) -> syn::Result<TokenStream> {
    let type_name = match *input.self_ty {
        syn::Type::Path(ref type_path) => type_path.path.segments.last().unwrap().ident.clone(),
        ref other => {
            return Err(syn::Error::new_spanned(
                input.self_ty.clone(),
                format!("Invalid RPC type `{}`", quote! { #other }),
            ))
        }
    };

    inner_rpc_gen(attr_contents, &input, &type_name)
}

mod utils {
    use syn::Meta;

    use super::*;

    pub fn add_attr_meta_if_missing(attrs: &mut Vec<Meta>, new_meta: Meta) {
        if attrs.iter().any(|attr| match attr {
            Meta::Path(path) => path == new_meta.path(),
            _ => false,
        }) {
            return;
        }

        attrs.push(new_meta);
    }

    pub fn function_arg_names(signature: &Signature) -> syn::Result<Vec<Ident>> {
        signature
            .inputs
            .iter()
            .map(|item| match item {
                FnArg::Receiver(arg) => Ok(Ident::new("self", arg.self_token.span)),
                FnArg::Typed(PatType { pat, .. }) => {
                    if let syn::Pat::Ident(syn::PatIdent { ident, .. }) = &**pat {
                        Ok(ident.clone())
                    } else {
                        Err(syn::Error::new_spanned(pat, format!("All arguments to this function must be named and must bind a variable. `{}` is not a valid argument", quote! { #pat })))
                    }
                }
            })
            .collect()
    }

    pub struct RpcMethodAttribute {
        pub attr: Attribute,
        pub index_within_attrs: usize,
    }

    impl RpcMethodAttribute {
        // Returns an attribute with the name `rpc_method` replaced with `method`, and the index
        /// into the argument array where the attribute was found.
        pub fn parse(method: &syn::ImplItemFn) -> syn::Result<Self> {
            for (idx, attribute) in method.attrs.iter().enumerate() {
                if let Meta::List(syn::MetaList {
                    path,
                    delimiter,
                    tokens,
                }) = &attribute.meta
                {
                    if path.is_ident("rpc_method") {
                        // Create a new attribute with the modified path
                        let new_attr = Attribute {
                            meta: Meta::List(syn::MetaList {
                                path: parse_quote!(method),
                                delimiter: delimiter.clone(),
                                tokens: tokens.clone(),
                            }),
                            ..attribute.clone()
                        };

                        return Ok(Self {
                            attr: new_attr,
                            index_within_attrs: idx,
                        });
                    }
                }
            }

            Err(syn::Error::new_spanned(
                method,
                "Missing `#[rpc_method]` attribute to function",
            ))
        }

        fn tokens(&self) -> &TokenStream {
            let Meta::List(list) = &self.attr.meta else {
                unreachable!("`RpcMethodAttribute::parse` only accepts list attributes");
            };
            &list.tokens
        }

        /// Whether the attribute sets jsonrpsee's `with_extensions` flag.
        pub fn has_with_extensions(&self) -> bool {
            self.tokens().clone().into_iter().any(|tt| {
                matches!(tt, proc_macro2::TokenTree::Ident(ident) if ident == "with_extensions")
            })
        }

        /// Adds jsonrpsee's `with_extensions` flag to the attribute, unless it's already there.
        pub fn ensure_with_extensions(&mut self) {
            if self.has_with_extensions() {
                return;
            }
            let needs_comma = self.tokens().clone().into_iter().last().is_some_and(
                |tt| !matches!(tt, proc_macro2::TokenTree::Punct(punct) if punct.as_char() == ','),
            );
            let Meta::List(list) = &mut self.attr.meta else {
                unreachable!("`RpcMethodAttribute::parse` only accepts list attributes");
            };
            if needs_comma {
                list.tokens.extend(quote! { , });
            }
            list.tokens.extend(quote! { with_extensions });
        }
    }

    pub struct ApiStateAccessorArg {
        pub ty: syn::Type,
        pub ident: Ident,
        pub idx: usize,
    }

    impl ApiStateAccessorArg {
        pub fn parse(sig: &Signature) -> syn::Result<Option<Self>> {
            let error = || {
                syn::Error::new_spanned(
                    sig,
                    "The `api_state_accessor` argument to the `#[rpc_method]`-annotated function has the wrong type. It should be a mutable reference to a `ApiStateAccessor<...>`. Either fix the type or remove the `api_state_accessor` argument.",
                )
            };

            for (idx, input) in sig.inputs.iter().enumerate() {
                // We're not interested in `self`.
                let FnArg::Typed(PatType { ty, pat, .. }) = input else {
                    continue;
                };

                // We're only interested in arguments that bind a new variable.
                let syn::Pat::Ident(syn::PatIdent { ident, .. }) = *pat.clone() else {
                    continue;
                };

                if ident != "state" && ident != "_state" {
                    continue;
                }

                // It should be a reference...
                let syn::Type::Reference(syn::TypeReference {
                    elem, mutability, ..
                }) = *ty.clone()
                else {
                    return Err(error());
                };

                // ...to a `syn::Type`!
                let syn::Type::Path(syn::TypePath { path, .. }) = elem.as_ref() else {
                    return Err(error());
                };

                // It must be a *mutable* reference!
                if mutability.is_none() {
                    return Err(error());
                }

                // Let's inspect the type path.
                let Some(segment) = path.segments.last() else {
                    return Err(error());
                };

                // It must have generic arguments.
                let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
                    return Err(error());
                };

                // It must have exactly *one* generic argument.
                if args.args.len() != 1 {
                    return Err(error());
                }

                // Finally.
                return Ok(Some(Self {
                    ty: *elem.clone(),
                    ident,
                    idx,
                }));
            }
            Ok(None)
        }
    }

    /// The `&jsonrpsee::Extensions` argument of an `#[rpc_method]`-annotated function, whatever its name.
    /// It receives the extensions of the JSON-RPC request, such as data inserted by HTTP middleware.
    pub struct ExtensionsArg {
        pub idx: usize,
    }

    impl ExtensionsArg {
        pub fn parse(sig: &Signature) -> syn::Result<Option<Self>> {
            for (idx, input) in sig.inputs.iter().enumerate() {
                let FnArg::Typed(PatType { ty, .. }) = input else {
                    continue;
                };
                // RPC parameters are deserialized into owned values, so a reference to a type
                // named `Extensions` can only be the request extensions.
                let syn::Type::Reference(syn::TypeReference {
                    elem, mutability, ..
                }) = ty.as_ref()
                else {
                    continue;
                };
                let syn::Type::Path(syn::TypePath { path, .. }) = elem.as_ref() else {
                    continue;
                };
                if path
                    .segments
                    .last()
                    .is_none_or(|segment| segment.ident != "Extensions")
                {
                    continue;
                }
                if mutability.is_some() {
                    return Err(syn::Error::new_spanned(
                        input,
                        "The `Extensions` argument to the `#[rpc_method]`-annotated function must be a shared reference: `&jsonrpsee::Extensions`.",
                    ));
                }

                return Ok(Some(Self { idx }));
            }
            Ok(None)
        }
    }
}
