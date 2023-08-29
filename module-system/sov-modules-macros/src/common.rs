use std::collections::HashMap;

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, ToTokens};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    DataStruct, Fields, GenericArgument, GenericParam, ImplGenerics, Meta, PathArguments,
    PathSegment, TypeGenerics, TypeParamBound,
};

#[derive(Clone)]
pub(crate) struct StructNamedField {
    pub(crate) ident: proc_macro2::Ident,
    pub(crate) ty: syn::Type,
    pub(crate) attrs: Vec<syn::Attribute>,
    pub(crate) vis: syn::Visibility,
}

impl StructNamedField {
    #[cfg_attr(not(feature = "native"), allow(unused))]
    pub(crate) fn filter_attrs(&mut self, filter: impl FnMut(&syn::Attribute) -> bool) {
        self.attrs = std::mem::take(&mut self.attrs)
            .into_iter()
            .filter(filter)
            .collect();
    }
}

impl ToTokens for StructNamedField {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let docs = &self.attrs;
        let vis = &self.vis;
        let ident = &self.ident;
        let ty = &self.ty;
        tokens.extend(quote::quote! {
            #( #docs )*
            #vis #ident: #ty
        });
    }
}

pub(crate) struct StructFieldExtractor {
    macro_name: &'static str,
}

impl StructFieldExtractor {
    pub(crate) fn new(macro_name: &'static str) -> Self {
        Self { macro_name }
    }

    // Extracts named fields form a struct or emits an error.
    pub(crate) fn get_fields_from_struct(
        &self,
        data: &syn::Data,
    ) -> Result<Vec<StructNamedField>, syn::Error> {
        match data {
            syn::Data::Struct(data_struct) => self.get_fields_from_data_struct(data_struct),
            syn::Data::Enum(en) => Err(syn::Error::new_spanned(
                en.enum_token,
                format!("The {} macro supports structs only.", self.macro_name),
            )),
            syn::Data::Union(un) => Err(syn::Error::new_spanned(
                un.union_token,
                format!("The {} macro supports structs only.", self.macro_name),
            )),
        }
    }

    /// Extract the named fields from a struct, or generate named fields matching the fields of an unnamed struct.
    /// Names follow the pattern `field0`, `field1`, etc.
    #[cfg_attr(not(feature = "native"), allow(unused))]
    pub(crate) fn get_or_generate_named_fields(fields: &Fields) -> Vec<StructNamedField> {
        match fields {
            Fields::Unnamed(unnamed_fields) => unnamed_fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    let ident = Ident::new(&format!("field{}", i), field.span());
                    let ty = &field.ty;
                    StructNamedField {
                        attrs: field.attrs.clone(),
                        vis: field.vis.clone(),
                        ident,
                        ty: ty.clone(),
                    }
                })
                .collect::<Vec<_>>(),
            Fields::Named(fields_named) => fields_named
                .named
                .iter()
                .map(|field| {
                    let ty = &field.ty;
                    StructNamedField {
                        attrs: field.attrs.clone(),
                        vis: field.vis.clone(),
                        ident: field.ident.clone().expect("Named fields must have names!"),
                        ty: ty.clone(),
                    }
                })
                .collect::<Vec<_>>(),
            Fields::Unit => Vec::new(),
        }
    }

    fn get_fields_from_data_struct(
        &self,
        data_struct: &DataStruct,
    ) -> Result<Vec<StructNamedField>, syn::Error> {
        let mut output_fields = Vec::default();

        for original_field in data_struct.fields.iter() {
            let field_ident = original_field
                .ident
                .as_ref()
                .ok_or(syn::Error::new_spanned(
                    &original_field.ident,
                    format!(
                        "The {} macro supports structs only, unnamed fields witnessed.",
                        self.macro_name
                    ),
                ))?;

            let field = StructNamedField {
                ident: field_ident.clone(),
                ty: original_field.ty.clone(),
                attrs: original_field.attrs.clone(),
                vis: original_field.vis.clone(),
            };

            output_fields.push(field);
        }
        Ok(output_fields)
    }
}

pub(crate) const CALL: &str = "Call";

/// Represents "parsed" rust struct.
pub(crate) struct StructDef<'a> {
    pub(crate) ident: proc_macro2::Ident,
    pub(crate) impl_generics: ImplGenerics<'a>,
    pub(crate) type_generics: TypeGenerics<'a>,
    pub(crate) generic_param: &'a Ident,
    pub(crate) fields: Vec<StructNamedField>,
    pub(crate) where_clause: Option<&'a syn::WhereClause>,
}

impl<'a> StructDef<'a> {
    pub(crate) fn new(
        ident: proc_macro2::Ident,
        fields: Vec<StructNamedField>,
        impl_generics: ImplGenerics<'a>,
        type_generics: TypeGenerics<'a>,
        generic_param: &'a Ident,
        where_clause: Option<&'a syn::WhereClause>,
    ) -> Self {
        Self {
            ident,
            fields,
            impl_generics,
            type_generics,
            generic_param,
            where_clause,
        }
    }

    /// Creates an enum type based on the underlying struct.
    pub(crate) fn create_enum(
        &self,
        enum_legs: &[proc_macro2::TokenStream],
        postfix: &'static str,
        serialization_attrs: &Vec<TokenStream>,
    ) -> proc_macro2::TokenStream {
        let enum_ident = self.enum_ident(postfix);
        let impl_generics = &self.impl_generics;
        let where_clause = &self.where_clause;

        quote::quote! {
            #[allow(non_camel_case_types)]
            #[derive(::core::fmt::Debug, PartialEq, #(#serialization_attrs),*)]
            pub enum #enum_ident #impl_generics #where_clause {
                #(#enum_legs)*
            }
        }
    }

    pub(crate) fn enum_ident(&self, postfix: &'static str) -> Ident {
        let ident = &self.ident;
        format_ident!("{ident}{postfix}")
    }
}

/// Gets the type parameter's identifier from [`syn::Generics`].
pub(crate) fn get_generics_type_param(
    generics: &syn::Generics,
    error_span: Span,
) -> Result<Ident, syn::Error> {
    let generic_param = match generics
        .params
        .first()
        .ok_or_else(|| syn::Error::new(error_span, "No generic parameters found"))?
    {
        GenericParam::Type(ty) => &ty.ident,
        GenericParam::Lifetime(lf) => {
            return Err(syn::Error::new_spanned(
                lf,
                "Lifetime parameters not supported.",
            ))
        }
        GenericParam::Const(cnst) => {
            return Err(syn::Error::new_spanned(
                cnst,
                "Const parameters not supported.",
            ))
        }
    };

    Ok(generic_param.clone())
}

pub(crate) fn get_attribute_values(
    item: &syn::DeriveInput,
    attribute_name: &str,
) -> Vec<TokenStream> {
    let mut values = vec![];

    // Find the attribute with the given name on the root item
    item.attrs
        .iter()
        .filter(|attr| attr.path.is_ident(attribute_name))
        .for_each(|attr| {
            if let Ok(Meta::List(list)) = attr.parse_meta() {
                values.extend(list.nested.iter().map(|n| {
                    let mut tokens = TokenStream::new();
                    n.to_tokens(&mut tokens);
                    tokens
                }));
            }
        });

    values
}

pub(crate) fn get_serialization_attrs(
    item: &syn::DeriveInput,
) -> Result<Vec<TokenStream>, syn::Error> {
    const SERIALIZE: &str = "Serialize";
    const DESERIALIZE: &str = "Deserialize";

    let serialization_attrs = get_attribute_values(item, "serialization");

    let mut has_serialize = false;
    let mut has_deserialize = false;
    let mut has_other = false;

    let attributes: Vec<String> = serialization_attrs.iter().map(|t| t.to_string()).collect();

    for attr in &attributes {
        if attr.contains(SERIALIZE) {
            has_serialize = true;
        } else if attr.contains(DESERIALIZE) {
            has_deserialize = true;
        } else {
            has_other = true;
        }
    }

    let tokens: TokenStream = quote::quote! { serialization };
    if !has_serialize || !has_deserialize {
        return Err(syn::Error::new_spanned(
            &tokens,
            format!(
                "Serialization attributes must contain both '{}' and '{}', but contains '{:?}'",
                SERIALIZE, DESERIALIZE, &attributes
            ),
        ));
    } else if has_other {
        return Err(syn::Error::new_spanned(
            &tokens,
            format!("Serialization attributes can not contain attributes that are not '{}' and '{}', but contains: '{:?}'", 
                SERIALIZE, DESERIALIZE, &attributes.iter().filter(|a| !a.contains(SERIALIZE) && !a.contains(DESERIALIZE)).collect::<Vec<_>>()),
        ));
    }

    Ok(serialization_attrs)
}

/// Extract a mapping from generic types to their associated trait bounds, including
/// the ones from the where clause.
///
/// For example, given the following struct:
/// ```rust,ignore
/// use sov_modules_macros::common::GenericTypesWithBounds;
/// let test_struct: syn::ItemStruct = syn::parse_quote! {
///     struct TestStruct<T: SomeTrait> where T: SomeOtherTrait {
///         field: T
///     }
/// };
/// // We want to extract both the inline bounds, and the bounds from the where clause...
/// // so that the generics from above definition are equivalent what we would have gotten
/// // from writing `T: SomeTrait + SomeOtherTrait` inline
/// let desired_bounds_for_t: syn::TypeParam = syn::parse_quote!(T: SomeTrait + SomeThirdTrait);
///
/// // That is exactly what `GenericTypesWithBounds` does
/// let our_bounds = extract_generic_type_bounds(&test_struct.generics);
/// assert_eq!(our_bounds.get(T), Some(&desired_bounds_for_t.bounds));
/// ```
/// TODO: DO WE REALLY NEED TYPE_PATH? WOULDN'T IT BE ENOUGH TO JUST HAVE THE PATH?
#[cfg_attr(not(feature = "native"), allow(unused))]
pub(crate) fn extract_generic_type_bounds(
    generics: &syn::Generics,
) -> HashMap<syn::Path, Punctuated<TypeParamBound, syn::token::Add>> {
    let mut generics_with_bounds: HashMap<_, _> = Default::default();
    // Collect the inline bounds from each generic param
    for param in generics.params.iter() {
        if let GenericParam::Type(ty) = param {
            let path_segment = PathSegment {
                ident: ty.ident.clone(),
                arguments: syn::PathArguments::None,
            };
            let path = syn::Path {
                leading_colon: None,
                segments: Punctuated::from_iter(vec![path_segment]),
            };
            generics_with_bounds.insert(path, ty.bounds.clone());
        }
    }

    // Iterate over the bounds in the `where_clause` and add them to the map
    if let Some(where_clause) = &generics.where_clause {
        for predicate in &where_clause.predicates {
            // We can ignore lifetimes and "Eq" predicates since they don't add any trait bounds
            // so just match on `Type` predicates
            if let syn::WherePredicate::Type(predicate_type) = predicate {
                // If the bounded type is a regular type path, we need to extract the bounds and add them to the map.
                // For now, we ignore more exotic bounds `[T; N]: SomeTrait`.
                if let syn::Type::Path(type_path) = &predicate_type.bounded_ty {
                    match generics_with_bounds.entry(type_path.path.clone()) {
                        std::collections::hash_map::Entry::Occupied(mut entry) => {
                            entry.get_mut().extend(predicate_type.bounds.clone())
                        }
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(predicate_type.bounds.clone());
                        }
                    }
                }
            }
        }
    }
    generics_with_bounds
}

#[derive(Default)]
pub struct GenericWithMatchingPathArguments {
    pub path_arguments: syn::PathArguments,
    pub generics: syn::Generics,
}

/// For example, given the following struct:
/// ```rust,ignore
/// struct MyStruct<T: SomeTrait, U: SomeOtherTrait> {
///    field1: PhantomData<T>,
///    field2: Vec<U>
/// }
/// ```
///
/// This function will return a `syn::Generics` corresponding to `<T: SomeTrait>` when
/// invoked on the PathArguments for field1.
#[cfg_attr(not(feature = "native"), allow(unused))]
pub(crate) fn generics_for_field(
    outer_generics: &syn::Generics,
    field_generic_types: &PathArguments,
) -> GenericWithMatchingPathArguments {
    let generic_bounds = extract_generic_type_bounds(outer_generics);
    match field_generic_types {
        PathArguments::AngleBracketed(angle_bracketed_data) => {
            let mut args_with_bounds = Punctuated::<GenericParam, syn::token::Comma>::new();
            let mut where_predicates = Punctuated::<syn::WherePredicate, syn::token::Comma>::new();
            let mut new_generic_args = Punctuated::<syn::GenericArgument, syn::token::Comma>::new();
            for generic_arg in &angle_bracketed_data.args {
                if let syn::GenericArgument::Type(syn::Type::Path(type_path)) = generic_arg {
                    let last_segment = type_path
                        .path
                        .segments
                        .last()
                        .expect("Type path must have at least one segment")
                        .clone();

                    let ident = &last_segment.ident;

                    let bounds = generic_bounds
                        .get(&type_path.path)
                        .cloned()
                        .unwrap_or_default();

                    let last: syn::TypePath = syn::TypePath {
                        qself: None,
                        path: last_segment.clone().into(),
                    };

                    // AngleBracketedGenericArguments -> Punctuated<GenericArgument -> Type -> TypePath

                    new_generic_args.push(GenericArgument::Type(syn::Type::Path(last)));

                    // let sub_path_segment_start = &type_path
                    //     .path
                    //     .segments
                    //     .first()
                    //     .expect("Type path must have at least one segment")
                    //     .clone();
                    // let sub_path: syn::Path = sub_path_segment_start.clone().into();
                    //
                    // let bounds = generic_bounds.get(&sub_path).cloned().unwrap_or_default();
                    //
                    if type_path.path.segments.len() > 1 {
                        let where_bounds = generic_bounds
                            .get(&type_path.path)
                            .cloned()
                            .unwrap_or_default();

                        if !where_bounds.is_empty() {
                            let sub_type_path = syn::TypePath {
                                // This needs to be addressed, from docs:
                                // A path like `std::slice::Iter`, optionally qualified with a
                                // self-type as in `<Vec<T> as SomeTrait>::Associated`.
                                qself: None,
                                path: type_path.path.clone(),
                            };

                            let where_predicate = syn::WherePredicate::Type(syn::PredicateType {
                                lifetimes: None,
                                bounded_ty: syn::Type::Path(sub_type_path),
                                colon_token: Default::default(),
                                bounds: where_bounds,
                            });

                            where_predicates.push(where_predicate);
                        }
                    }

                    // Construct a "type param" with the appropriate bounds. This corresponds to a syntax
                    // tree like `T: Trait1 + Trait2`

                    let generic_type_param_with_bounds = syn::TypeParam {
                        attrs: Vec::new(),
                        ident: ident.clone(),
                        colon_token: Some(syn::token::Colon {
                            spans: [type_path.span()],
                        }),
                        bounds: bounds.clone(),
                        eq_token: None,
                        default: None,
                    };
                    args_with_bounds.push(GenericParam::Type(generic_type_param_with_bounds))
                }
            }
            // Construct a `Generics` struct with the generic type parameters and their bounds.
            // This corresponds to a syntax tree like `<T: Trait1 + Trait2>`

            let where_clause = if where_predicates.is_empty() {
                None
            } else {
                Some(syn::WhereClause {
                    where_token: Default::default(),
                    predicates: where_predicates,
                })
            };

            let path_arguments =
                syn::PathArguments::AngleBracketed(syn::AngleBracketedGenericArguments {
                    colon2_token: None,
                    lt_token: Default::default(),
                    args: new_generic_args,
                    gt_token: Default::default(),
                });

            let generics = syn::Generics {
                lt_token: Some(syn::token::Lt {
                    spans: [field_generic_types.span()],
                }),
                params: args_with_bounds,
                gt_token: Some(syn::token::Gt {
                    spans: [field_generic_types.span()],
                }),
                where_clause,
            };

            GenericWithMatchingPathArguments {
                path_arguments,
                generics,
            }
        }
        // We don't need to do anything if the generic type parameters are not angle bracketed
        _ => Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use quote::ToTokens;
    use syn::parse_quote;

    use crate::common::{extract_generic_type_bounds, generics_for_field};

    #[test]
    fn test_generic_types_with_bounds() {
        let test_struct: syn::ItemStruct = syn::parse_quote! {
            struct TestStruct<T: SomeTrait, U: SomeOtherTrait, V> where T: SomeThirdTrait {
                field: (T, U, V)
            }
        };
        let generics = test_struct.generics;
        let our_bounds = extract_generic_type_bounds(&generics);
        let expected_bounds_for_t: syn::TypeParam =
            syn::parse_quote!(T: SomeTrait + SomeThirdTrait);
        let expected_bounds_for_u: syn::TypeParam = syn::parse_quote!(U: SomeOtherTrait);

        assert_eq!(
            our_bounds.get(&parse_quote!(T)),
            Some(&expected_bounds_for_t.bounds)
        );
        assert_eq!(
            our_bounds.get(&parse_quote!(U)),
            Some(&expected_bounds_for_u.bounds)
        );
        assert_eq!(
            our_bounds.get(&parse_quote!(V)),
            Some(&syn::punctuated::Punctuated::new())
        );
    }

    #[test]
    fn test_generic_types_with_associated_type_bounds() {
        let test_struct: syn::ItemStruct = syn::parse_quote! {
            struct TestStruct<T: SomeTrait, U: SomeOtherTrait, V> where T::Error: Debug {
                field: (T, U, V)
            }
        };
        let generics = test_struct.generics;
        let our_bounds = extract_generic_type_bounds(&generics);
        let expected_bounds_for_t: syn::TypeParam = syn::parse_quote!(T: SomeTrait);
        let expected_bounds_for_t_error: syn::WherePredicate = syn::parse_quote!(T::Error: Debug);
        if let syn::WherePredicate::Type(expected_bounds_for_t_error) = expected_bounds_for_t_error
        {
            assert_eq!(
                our_bounds.get(&parse_quote!(T::Error)),
                Some(&expected_bounds_for_t_error.bounds)
            );
        } else {
            unreachable!("Expected a type predicate")
        };
        let expected_bounds_for_u: syn::TypeParam = syn::parse_quote!(U: SomeOtherTrait);

        assert_eq!(
            our_bounds.get(&parse_quote!(T)),
            Some(&expected_bounds_for_t.bounds)
        );

        assert_eq!(
            our_bounds.get(&parse_quote!(U)),
            Some(&expected_bounds_for_u.bounds)
        );
        assert_eq!(
            our_bounds.get(&parse_quote!(V)),
            Some(&syn::punctuated::Punctuated::new())
        );
    }

    #[test]
    fn how_to_flatten_arguments() {
        let associated_path_arguments: syn::AngleBracketedGenericArguments =
            parse_quote! { <C, T::Error> };

        for arg in associated_path_arguments.args.iter() {
            println!("ARG: {:?}", arg);
        }

        let flattened_arguments: syn::AngleBracketedGenericArguments = parse_quote! { <C, Error> };
    }

    #[test]
    fn test_generics_for_field_associated_type() {
        let test_struct: syn::ItemStruct = parse_quote! {
            struct Runtime<C: Context, D: SomeOtherTrait, V> where V: SomeThirdTrait, T::Error: Debug {
                module_1: Module1<C, T::Error>,
                module_2: Module2<C, D>,
                module_3: Module3<C, V>,
            }
        };

        let generics: syn::Generics = test_struct.generics;
        let path_arguments: syn::AngleBracketedGenericArguments = parse_quote! { <C, T::Error> };

        let p = syn::PathArguments::AngleBracketed(path_arguments);

        let GenericWithMatchingPathArguments {
            generics: actual_generics,
            ..
        } = generics_for_field(&generics, &p);

        let expected: syn::ItemStruct = parse_quote! {
            struct Module1Generated<C: Context, Error: Debug>  {
                module_1: Module1<C, Error>,
            }
        };
        println!(
            "ACTUAL  : '{}' WHERE: '{}'",
            actual_generics.to_token_stream(),
            actual_generics
                .where_clause
                .map_or("".to_string(), |w| w.to_token_stream().to_string())
        );
        println!(
            "EXPECTED: '{}' WHERE: '{}'",
            expected.generics.to_token_stream(),
            expected
                .generics
                .where_clause
                .map_or("".to_string(), |w| w.to_token_stream().to_string())
        );

        // ACTUAL  : '< C : Context , T >' WHERE: 'where T :: Error : Debug'
        // EXPECTED: '< C : Context , Error : Debug >' WHERE: ''

        // assert_eq!(expected.generics, actual_generics);
    }

    #[test]
    fn test_generics_for_field_associated_types_nested_as() {
        let test_struct: syn::ItemStruct = parse_quote! {
            struct TestStruct<T: FirstTrait, U: SecondTrait, V>
                where
                    V: ThirdTrait,
                    <U as SecondTrait>::Message: Message,
                    <<U as SecondTrait>::Message as Message>::Caller: std::fmt::Display,
                    T::Error: std::fmt::Debug,
            {
                field1: Foo<U, T::Error>,
                field2: Bar<<U::Message as Message>::Caller, V>,
            }
        };

        let generics: syn::Generics = test_struct.generics;
        let path_arguments: syn::AngleBracketedGenericArguments =
            parse_quote! { <<U::Message as Message>::Caller, V> };

        let p = syn::PathArguments::AngleBracketed(path_arguments);

        let GenericWithMatchingPathArguments {
            generics: actual_generics,
            ..
        } = generics_for_field(&generics, &p);

        let expected: syn::ItemStruct = parse_quote! {
            struct Field1Generated<U: SecondTrait, V>
                where
                    V: ThirdTrait,
                    <U as SecondTrait>::Message: Message,
                    <<U as SecondTrait>::Message as Message>::Caller: std::fmt::Display,
            {
                field_1: Bar<U, T::Error>,
            }
        };

        println!(
            "ACTUAL  : '{}' WHERE: '{}'",
            actual_generics.to_token_stream(),
            actual_generics
                .where_clause
                .map_or("".to_string(), |w| w.to_token_stream().to_string())
        );
        println!(
            "EXPECTED: '{}' WHERE: '{}'",
            expected.generics.to_token_stream(),
            expected
                .generics
                .where_clause
                .map_or("".to_string(), |w| w.to_token_stream().to_string())
        );

        // assert_eq!(expected.generics, actual_generics);
    }

    #[test]
    fn test_generic_types_with_associated_type_bounds_2() {
        // where T::Error: Debug
        let test_struct: syn::ItemStruct = syn::parse_quote! {
            struct TestStruct<T: SomeTrait, U: SomeOtherTrait, V, X> where T::Error: Debug, X::Thing: std::fmt::Display {
                field: (T::Error, U, V),
                fiel2: X::Thing,
            }
        };
        let generics = test_struct.generics;
        let our_bounds = extract_generic_type_bounds(&generics);
        for (t, p) in our_bounds {
            println!("{} _::_ {}", t.to_token_stream(), p.to_token_stream());
        }
        // U _::_ SomeOtherTrait
        // T _::_ SomeTrait
        // X _::_
        // T :: Error _::_ Debug
        // V _::_
        // X :: Thing _::_ std :: fmt :: Display

        // TODO: Add checks
        // let expected_bounds_for_t: syn::TypeParam = syn::parse_quote!(T: SomeTrait);
    }

    // TODO: Test with deeper nesting.
    //         let expected: syn::ItemStruct = parse_quote! {
    //             struct Dummy<U: SomeOtherTrait, T: SomeTrait>  where T::Error: Debug, <T::Error> as SomeTrait: AnotherTrait {
    //                 field_1: Result<U, T::Error>,
    //             }
    //         };

    // // T::Foo::Bar
}
