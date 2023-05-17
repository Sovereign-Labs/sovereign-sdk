use proc_macro2::{Ident,Span};
use quote::{format_ident, quote, ToTokens};
use std::str::FromStr;
use syn::{Attribute, FnArg, ImplItem, Meta, MetaList, PatType, Path, PathSegment, Signature, Type, AngleBracketedGenericArguments, Data, DeriveInput, Field, GenericArgument, Lit, NestedMeta, parse_str, PathArguments, TypeParam, TypePath, Fields, FieldsNamed, parse_macro_input, Generics, TypeParamBound, Token, ItemImpl, parenthesized};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::parse::{Parse, ParseStream};


/// Returns an attribute with the name `rpc_method` replaced with `method`, and the index
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

fn jsonrpsee_rpc_macro_path() -> Path {
    let segments = vec![
        Ident::new("jsonrpsee", proc_macro2::Span::call_site()),
        Ident::new("proc_macros", proc_macro2::Span::call_site()),
        Ident::new("rpc", proc_macro2::Span::call_site()),
    ];

    let path_segments = segments.into_iter().map(|ident| PathSegment {
        ident,
        arguments: syn::PathArguments::None,
    });

    Path {
        leading_colon: Some(syn::Token![::](proc_macro2::Span::call_site())),
        segments: syn::punctuated::Punctuated::from_iter(path_segments),
    }
}

fn find_working_set_argument(sig: &Signature) -> Option<(usize, syn::Type)> {
    for (idx, input) in sig.inputs.iter().enumerate() {
        if let FnArg::Typed(PatType { ty, .. }) = input {
            if let syn::Type::Reference(syn::TypeReference { elem, .. }) = *ty.clone() {
                if let syn::Type::Path(syn::TypePath { path, .. }) = elem.as_ref() {
                    if let Some(segment) = path.segments.last() {
                        // TODO: enforce that the working set has exactly one angle bracketed argument
                        if segment.ident == "WorkingSet" && !segment.arguments.is_empty() {
                            return Some((idx, *elem.clone()));
                        }
                    }
                }
            }
        }
    }
    None
}

fn add_param_to_signature(signature: &mut Signature, working_set_type: &Type) {
    let working_set_ident = syn::Ident::new("working_set", Span::call_site());
    let pat: syn::Pat = syn::parse_quote! { #working_set_ident };
    let ty = syn::Type::Reference(syn::TypeReference {
        and_token: syn::token::And { spans: [Span::call_site()] },
        lifetime: None,
        mutability: Some(syn::token::Mut { span: Span::call_site() }),
        elem: Box::new(working_set_type.clone()),
    });
    let pat_type = syn::PatType { attrs: vec![],
        pat: Box::new(pat),
        colon_token: syn::token::Colon { spans: [Span::call_site()] },
        ty: Box::new(ty) };
    let arg = syn::FnArg::Typed(pat_type);
    signature.inputs.push(arg);
}

fn construct_working_set_ident(generic_ident: Ident) -> Type {
    let workingset_ident = Ident::new("WorkingSet", Span::call_site());
    let storage_ident = Ident::new("Storage", Span::call_site());

    let segment_storage = PathSegment {
        ident: storage_ident,
        arguments: PathArguments::None,
    };

    let path_c_storage = Path {
        leading_colon: None,
        segments: Punctuated::from_iter(vec![generic_ident.into(), segment_storage]),
    };

    let arguments = Punctuated::from_iter(vec![
        GenericArgument::Type(Type::Path(TypePath {
            qself: None,
            path: path_c_storage,
        })),
    ]);

    let segment_workingset = PathSegment {
        ident: workingset_ident,
        arguments: PathArguments::AngleBracketed(AngleBracketedGenericArguments {
            colon2_token: None,
            lt_token: Token![<](Span::call_site()),
            args: arguments,
            gt_token: Token![>](Span::call_site()),
        }),
    };

    let path_workingset = Path::from(segment_workingset);
    Type::Path(TypePath { qself: None, path: path_workingset })
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
    /// Builds the trait `_RpcImpl` That will be implemented by the runtime
    fn build_rpc_impl_trait(&self) -> proc_macro2::TokenStream {
        let type_name = &self.type_name;
        let generics = &self.generics;
        let generics_params = generics
            .params
            .iter()
            .map(|param| {
                if let syn::GenericParam::Type(syn::TypeParam { ident, .. }) = param {
                    return quote! { #ident };
                }
                unreachable!("Expected a type parameter")
            })
            .collect::<Vec<_>>();


        // let debug_var = quote! { println!("STUFF {:?}", #(#generics_params)*,); };
        // println!("DBG1: {}", debug_var);
        // let debug_var = quote! { println!("STUFF {:?}", #type_name) };
        // println!("DBG2: {}", debug_var);

        let mut blanket_impl_methods = vec![];
        let mut impl_trait_methods = vec![];

        let impl_trait_name = format_ident!("{}RpcImpl", self.type_name);

        for method in self.methods.iter() {
            // Extract the names of the formal arguments
            let arg_values = method
                .method_signature
                .inputs
                .clone()
                .into_iter()
                .map(|item| {
                    if let FnArg::Typed(PatType { pat, .. }) = item {
                        if let syn::Pat::Ident(syn::PatIdent { ident, .. }) = *pat {
                            return quote! { #ident };
                        }
                        unreachable!("Expected a pattern identifier")
                    } else {
                        quote! { self }
                    }
                });

            let mut signature = method.method_signature.clone();
            let method_name = &method.method_name;

            let impl_trait_method = if let Some(idx) = method.idx_of_working_set_arg {
                // If necessary, adjust the signature to remove the working set argument and replace it with one generated by the implementer.
                // Remove the "self" argument as well
                let pre_working_set_args = arg_values
                    .clone()
                    .take(idx)
                    .filter(|arg| arg.to_string() != quote! { self }.to_string());
                let post_working_set_args = arg_values
                    .clone()
                    .skip(idx + 1)
                    .filter(|arg| arg.to_string() != quote! { self }.to_string());
                let mut inputs: Vec<syn::FnArg> = signature.inputs.clone().into_iter().collect();
                inputs.remove(idx);

                signature.inputs = inputs.into_iter().collect();

                quote! {
                    #signature {
                        <#type_name <#(#generics_params)*,> as ::sov_modules_api::ModuleInfo>::new().#method_name(#(#pre_working_set_args,)* &mut Self::get_working_set(self), #(#post_working_set_args),* )
                    }
                }
            } else {
                // Remove the "self" argument, since the method is invoked on `self` using dot notation
                let arg_values = arg_values
                    .clone()
                    .filter(|arg| arg.to_string() != quote! { self }.to_string());
                quote! {
                    #signature {
                        <#type_name <#(#generics_params)*,> as ::sov_modules_api::ModuleInfo>::new().#method_name(#(#arg_values),*)
                    }
                }
            };

            impl_trait_methods.push(impl_trait_method);


            signature.output = wrap_in_jsonprsee_result(&signature.output);
            let blanket_impl_method = if let Some(idx) = method.idx_of_working_set_arg {
                // If necessary, adjust the signature to remove the working set argument.
                let pre_working_set_args = arg_values.clone().take(idx);
                let post_working_set_args = arg_values.clone().skip(idx + 1);
                quote! {
                    #signature {
                        Ok(<Self as #impl_trait_name < #(#generics_params)*, >>::#method_name(#(#pre_working_set_args,)* #(#post_working_set_args),* ))
                    }
                }
            } else {
                quote! {
                    #signature {
                        Ok(<Self as #impl_trait_name < #(#generics_params)*, >>::#method_name(#(#arg_values),*))
                    }
                }
            };

            blanket_impl_methods.push(blanket_impl_method);
        }

        let rpc_impl_trait = if let Some(ref working_set_type) = self.working_set_type {
            quote! {
                pub trait #impl_trait_name #generics {
                    fn get_working_set(&self) -> #working_set_type;
                    #(#impl_trait_methods)*
                }
            }
        } else {
            quote! {
                pub trait #impl_trait_name #generics {
                    #(#impl_trait_methods)*
                }
            }
        };

        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
        let blanket_impl_generics = quote! {
            #impl_generics
        }
        .to_string();
        let blanket_impl_generics_without_braces = proc_macro2::TokenStream::from_str(
            &blanket_impl_generics[1..blanket_impl_generics.len() - 1],
        )
        .expect("Failed to parse generics without braces as token stream");
        let rpc_server_trait_name = format_ident!("{}RpcServer", self.type_name);
        let blanket_impl = quote! {
            impl <MacroGeneratedTypeWithLongNameToAvoidCollisions: #impl_trait_name #ty_generics
            + Send
            + Sync
            + 'static,  #blanket_impl_generics_without_braces > #rpc_server_trait_name #ty_generics for MacroGeneratedTypeWithLongNameToAvoidCollisions #where_clause {
                #(#blanket_impl_methods)*
            }
        };

        quote! {
            #rpc_impl_trait
            #blanket_impl
        }
    }
}

fn wrap_in_jsonprsee_result(return_type: &syn::ReturnType) -> syn::ReturnType {
    let result_type: Type = match return_type {
        syn::ReturnType::Default => syn::parse_quote! { ::jsonrpsee::core::RpcResult<()> },
        syn::ReturnType::Type(_, ty) => syn::parse_quote! { ::jsonrpsee::core::RpcResult<#ty> },
    };
    syn::ReturnType::Type(
        syn::token::RArrow {
            spans: [proc_macro2::Span::call_site(); 2],
        },
        Box::new(result_type),
    )
}

fn build_rpc_trait(
    attrs: &proc_macro2::TokenStream,
    type_name: Ident,
    mut input: syn::ItemImpl,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let intermediate_trait_name = format_ident!("{}Rpc", type_name);

    let wrapped_attr_args = quote! {
        (#attrs)
    };
    let rpc_attribute = syn::Attribute {
        pound_token: syn::token::Pound {
            spans: [proc_macro2::Span::call_site()],
        },
        style: syn::AttrStyle::Outer,
        bracket_token: syn::token::Bracket {
            span: proc_macro2::Span::call_site(),
        },
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
                let mut intermediate_trait_inputs = method.sig.inputs.clone();
                let working_set_arg = find_working_set_argument(&method.sig);
                let idx_of_working_set_arg = if let Some((idx, ty)) = working_set_arg {
                    // Remove the working set argument from the intermediate trait signature
                    let mut inputs: Vec<syn::FnArg> =
                        intermediate_trait_inputs.into_iter().collect();
                    inputs.remove(idx);
                    intermediate_trait_inputs = inputs.into_iter().collect();

                    // Store the type of the working set argument for later reference
                    rpc_info.working_set_type = Some(ty);
                    Some(idx)
                } else {
                    None
                };
                rpc_info.methods.push(RpcEnabledMethod {
                    method_name: method.sig.ident.clone(),
                    method_signature: method.sig.clone(),
                    idx_of_working_set_arg,
                });

                // Remove the working set argument from the signature
                let mut intermediate_signature = method.sig.clone();
                intermediate_signature.inputs = intermediate_trait_inputs;

                intermediate_signature.output =
                    wrap_in_jsonprsee_result(&intermediate_signature.output);

                // Build the annotated signature for the intermediate trait
                let annotated_signature = quote! {
                    #attr
                    #intermediate_signature;
                };
                intermediate_trait_items.push(annotated_signature);

                let mut original_method = method.clone();
                original_method.attrs.remove(idx_of_rpc_attr);
                simplified_impl_items.push(ImplItem::Method(original_method));
                continue;
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
    let type_name = match *input.self_ty {
        syn::Type::Path(ref type_path) => &type_path.path.segments.last().unwrap().ident,
        _ => return Err(syn::Error::new_spanned(input.self_ty, "Invalid type")),
    };

    build_rpc_trait(&attrs, type_name.clone(), input)
}

fn extract_type_name_and_generics(ty: &Type) -> Option<(String, Vec<GenericArgument>)> {
    match ty {
        Type::Path(TypePath { path, .. }) => {
            path.segments.last().map(|segment| {
                let type_name = segment.ident.to_string();
                let generics = match &segment.arguments {
                    PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) => {
                        args.iter().map(|x| x.clone())
                            .collect::<Vec<GenericArgument>>()
                    }
                    _ => vec![],
                };
                (type_name, generics)
            })
        }
        _ => None,
    }
}

fn extract_type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(TypePath { path, .. }) => {
            path.segments.last().map(|segment| segment.ident.to_string())
        }
        _ => None,
    }
}


fn get_generic_matching_constraint(gens: &Generics, trait_bound: &Path) -> Option<Ident> {
    let trait_bound_last_segment = trait_bound.segments.last()?.ident.to_string();
    gens.type_params().find_map(|type_param| {
        for bound in &type_param.bounds {
            if let syn::TypeParamBound::Trait(ref trait_bound_ref) = bound {
                let bound_last_segment = trait_bound_ref.path.segments.last()?.ident.to_string();
                if bound_last_segment == trait_bound_last_segment {
                    return Some(type_param.ident.clone());
                }
            }
        }
        None
    })
}

fn get_first_generic_with_constraint(gens: &Generics) -> Option<Ident> {
    gens.type_params().find_map(|type_param| {
        let TypeParam { ident, bounds, .. } = type_param;

        if bounds.iter().any(|bound| match bound {
            TypeParamBound::Trait(_) => true,
            _ => false,
        }) {
            Some(ident.clone())
        } else {
            None
        }
    })
}

pub(crate) fn rpc_impls(input: DeriveInput) -> Result<proc_macro::TokenStream, syn::Error> {

    // removing parameter
    // // currently, we're just letting the storage name "ProverStorage" or "MockStorage" etc be
    // // passed in directly using an attribute. the reason is that runtime doesn't have any
    // // visibility into the actual storage type being used. would require some modifications
    // // to get around it. One useful thing would be to do this optionally and default to the
    // // most common kind of storage used.
    // let storage_parameter = input.attrs.iter().find_map(|attr| {
    //     if attr.path.is_ident("storage") {
    //         if let Ok(Meta::NameValue(name_value)) = attr.parse_meta() {
    //             if let Lit::Str(lit_str) = name_value.lit {
    //                 let storage_ident: Ident = parse_str(&lit_str.value()).unwrap();
    //                 return Some(storage_ident);
    //             }
    //         }
    //     }
    //     None
    // }).expect("Rpc derive macro requires a storage parameter");

    let struct_name = input.ident;
    let struct_generics = input.generics;
    let struct_generics_params: Vec<Ident> =
        struct_generics
            .type_params()
            .into_iter()
            .map(|x| x.ident.clone())
            .collect();

    // we're checking to see which generic param of the struct the "Context" trait bound applies to
    // I'm not sure if there's a better way to handle this. would be ideal to somehow pass in the actual
    // trait itself, but i'm not sure if a there's a way to do that
    let trait_path: Path = parse_str("::Context").unwrap();

    // the logic here is to find the first generic param that has the traitbound "Context" specified
    // failing that, the code looks for the first generic param that has ANY trait bound. we can
    // make this more robust going forward
    let generic_ident = get_generic_matching_constraint(&struct_generics, &trait_path);
    let generic_ident = match generic_ident {
        None => get_first_generic_with_constraint(&struct_generics).expect("no matches to extract generics for RPC"),
        Some(id) => id
    };
    let fields = if let Data::Struct(data_struct) = input.data {
        data_struct.fields
    } else {
        panic!("rpc macro is only valid for struct");
    };

    let impls = fields.into_iter().map(|field| {
        let (field_type_name, field_type_generics) =
            extract_type_name_and_generics(&field.ty)
                .expect("couldn't parse types in runtime");
        let rpc_impl_ident = Ident::new(&format!("{}InnerRpcImpl", field_type_name), field.span());
        let field_name = field.ident.as_ref().expect("must have named fields");
        let field_type = field.ty;

        // the actual impls being generated
        quote! {
            impl #struct_generics #rpc_impl_ident<#(#field_type_generics),*> for #struct_name <#(#struct_generics_params),*>  {
                fn get_backing_impl(&self) -> &#field_type {
                    &self.#field_name
                }
            }
        }
    });

    let output = quote! {
        #(#impls)*

    };
    Ok(output.into())
}


struct TypeList(pub Punctuated<Type, syn::token::Comma>);

impl Parse for TypeList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        parenthesized!(content in input);
        Ok(TypeList(content.parse_terminated(Type::parse)?))
    }
}

pub(crate) fn rpc_outer_impls(args: proc_macro2::TokenStream,
                              input: syn::ItemImpl,) -> Result<proc_macro::TokenStream, syn::Error> {
    let type_name = &input.self_ty;
    let generics = &input.generics;
    let attrs = &input.attrs;

    let args: syn::Type = syn::parse2(args).expect("Expected a valid type list");

    let mut output_tokens = proc_macro2::TokenStream::new();

    let types = match args {
        syn::Type::Tuple(tuple) => tuple.elems,
        _ => panic!("Expected a tuple of types"),
    };

    let original_impl = quote! {
        #(#attrs)*
        #input
    };
    output_tokens.extend(original_impl);

    for arg in types {
        let mut trait_type_path = match arg {
            syn::Type::Path(type_path) => type_path.clone(),
            _ => panic!("Expected a path type"),
        };

        let last_segment = trait_type_path.path.segments.last_mut().unwrap();
        let context_type = match last_segment.arguments {
            syn::PathArguments::AngleBracketed(ref args) => {
                match args.args.first().expect("Expected at least one type argument") {
                    syn::GenericArgument::Type(syn::Type::Path(ref type_path)) => {
                        // Assuming type path has only one segment
                        type_path.path.segments.first().expect("Expected at least one segment").ident.clone()
                    }
                    _ => panic!("Expected a type argument"),
                }
            }
            _ => panic!("Expected angle bracketed arguments"),
        };

        last_segment.ident = syn::Ident::new(&format!("{}RpcImpl", last_segment.ident), last_segment.ident.span());

        let output = quote! {
            impl #trait_type_path for RpcStorage<#context_type>
            {
                fn get_working_set(&self) -> WorkingSet<<#context_type as Spec>::Storage> {
                    WorkingSet::new(self.storage.clone())
                }
            }

        };

        output_tokens.extend(output);
    }

    Ok(output_tokens.into())
}






