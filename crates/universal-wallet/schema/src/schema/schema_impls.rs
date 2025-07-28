#[doc(hidden)]
// This module contains the implementations of the `UniversalWallet` trait for all primitive types
mod primitive_type_impls {
    extern crate alloc;
    use std::any::TypeId;
    use std::cell::{Cell, RefCell, UnsafeCell};
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::marker::PhantomData;
    use std::ops::Range;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex, RwLock};

    use crate::schema::container::{Container, StructWithSerde};
    use crate::schema::{
        IndexLinking, Item, Link, OverrideSchema, Primitive, Schema, UniversalWallet,
    };
    use crate::ty::{
        macro_for_ints, ByteDisplay, ContainerSerdeMetadata, FieldOrVariantSerdeMetadata,
        IntegerDisplay, IntegerType, NamedField, Struct, Tuple, UnnamedField,
    };

    macro_rules! impl_for_int {
        ($t:ident) => {
            impl UniversalWallet for $t {
                fn scaffold() -> Item<IndexLinking> {
                    Item::Atom(Primitive::Integer(IntegerType::$t, IntegerDisplay::Decimal))
                }
                fn get_child_links(_schema: &mut Schema) -> Vec<Link> {
                    Vec::new()
                }
            }
        };
    }
    macro_for_ints!(impl_for_int);

    impl OverrideSchema for usize {
        type Output = u32;
    }

    impl OverrideSchema for isize {
        type Output = u32;
    }

    impl UniversalWallet for bool {
        fn scaffold() -> Item<IndexLinking> {
            Item::Atom(Primitive::Boolean)
        }
        fn get_child_links(_schema: &mut Schema) -> Vec<Link> {
            Vec::new()
        }
    }

    impl UniversalWallet for f32 {
        fn scaffold() -> Item<IndexLinking> {
            Item::Atom(Primitive::Float32)
        }
        fn get_child_links(_schema: &mut Schema) -> Vec<Link> {
            Vec::new()
        }
    }

    impl UniversalWallet for f64 {
        fn scaffold() -> Item<IndexLinking> {
            Item::Atom(Primitive::Float64)
        }
        fn get_child_links(_schema: &mut Schema) -> Vec<Link> {
            Vec::new()
        }
    }

    impl UniversalWallet for () {
        fn scaffold() -> Item<IndexLinking> {
            Item::Atom(Primitive::Skip { len: 0 })
        }
        fn get_child_links(_schema: &mut Schema) -> Vec<Link> {
            Vec::new()
        }
    }

    impl<T: 'static> UniversalWallet for PhantomData<T> {
        fn scaffold() -> Item<IndexLinking> {
            Item::Atom(Primitive::Skip { len: 0 })
        }
        fn get_child_links(_schema: &mut Schema) -> Vec<Link> {
            Vec::new()
        }
    }

    impl<T: UniversalWallet> UniversalWallet for Vec<T> {
        fn get_child_links(schema: &mut Schema) -> Vec<Link> {
            vec![T::make_linkable(schema)]
        }

        fn scaffold() -> Item<IndexLinking> {
            if TypeId::of::<T>() == TypeId::of::<u8>() {
                Item::Atom(Primitive::ByteVec {
                    display: ByteDisplay::Hex,
                })
            } else {
                Item::Container(Container::Vec {
                    value: Link::Placeholder,
                })
            }
        }
    }

    impl<T: UniversalWallet> OverrideSchema for HashSet<T> {
        type Output = Vec<T>;
    }

    impl<T: UniversalWallet> OverrideSchema for BTreeSet<T> {
        type Output = Vec<T>;
    }

    impl<const N: usize, T: UniversalWallet> UniversalWallet for [T; N] {
        fn get_child_links(schema: &mut Schema) -> Vec<Link> {
            vec![T::make_linkable(schema)]
        }

        fn scaffold() -> Item<IndexLinking> {
            if TypeId::of::<T>() == TypeId::of::<u8>() {
                Item::Atom(Primitive::ByteArray {
                    len: N,
                    display: ByteDisplay::Hex,
                })
            } else {
                Item::Container(Container::Array {
                    len: N,
                    value: Link::Placeholder,
                })
            }
        }
    }

    macro_rules! impl_container_type {
        ($t:ident) => {
            impl<T: UniversalWallet> OverrideSchema for $t<T> {
                type Output = T;
            }
        };
    }
    impl_container_type!(Box);
    impl_container_type!(Arc);
    impl_container_type!(Rc);
    impl_container_type!(Cell);
    impl_container_type!(RefCell);
    impl_container_type!(UnsafeCell);
    impl_container_type!(Mutex);
    impl_container_type!(RwLock);

    /// Helper macro, used for counting repetition without actually using the type
    macro_rules! type_to_placeholder {
        ($t: tt) => {
            Link::Placeholder
        };
    }
    macro_rules! impl_tuple_type {
        ($($tts:tt),*) => {
            impl<$($tts: UniversalWallet + 'static,)*> UniversalWallet for ($($tts,)*) {
                fn scaffold() -> Item<IndexLinking> {
                    Item::Container(Container::Tuple(Tuple {
                        template: None,
                        peekable: false,
                        fields: vec![
                            $(UnnamedField {
                                value: type_to_placeholder!($tts),
                                silent: false,
                                doc: "".to_string()
                            }),*
                        ]
                    }))
                }

                fn get_child_links(schema: &mut Schema) -> Vec<Link> {
                    vec![$($tts::make_linkable(schema)),*]
                }
            }
        };
    }
    // This is purely a convenience - any higher amount of tuples can simply be handled by the
    // derive macro.
    impl_tuple_type!(T1, T2);
    impl_tuple_type!(T1, T2, T3);
    impl_tuple_type!(T1, T2, T3, T4);
    impl_tuple_type!(T1, T2, T3, T4, T5);
    impl_tuple_type!(T1, T2, T3, T4, T5, T6);
    impl_tuple_type!(T1, T2, T3, T4, T5, T6, T7);
    impl_tuple_type!(T1, T2, T3, T4, T5, T6, T7, T8);

    impl<T: UniversalWallet> UniversalWallet for Option<T> {
        fn scaffold() -> Item<IndexLinking> {
            Item::Container(Container::Option {
                value: Link::Placeholder,
            })
        }

        fn get_child_links(schema: &mut Schema) -> Vec<Link> {
            vec![T::make_linkable(schema)]
        }
    }

    impl<K: UniversalWallet, V: UniversalWallet> UniversalWallet for HashMap<K, V> {
        fn scaffold() -> Item<IndexLinking> {
            Item::Container(Container::Map {
                key: Link::Placeholder,
                value: Link::Placeholder,
            })
        }

        fn get_child_links(schema: &mut Schema) -> Vec<Link> {
            vec![K::make_linkable(schema), V::make_linkable(schema)]
        }
    }

    impl<K: UniversalWallet, V: UniversalWallet> OverrideSchema for BTreeMap<K, V> {
        type Output = HashMap<K, V>;
    }

    impl<T: UniversalWallet> UniversalWallet for Range<T> {
        fn scaffold() -> Item<IndexLinking> {
            Item::Container(Container::Struct(StructWithSerde {
                ty: Struct {
                    type_name: "Range".to_string(),
                    template: Some("{}..{}".to_string()),
                    peekable: false,
                    fields: vec![
                        NamedField {
                            value: Link::Placeholder,
                            doc: "".to_string(),
                            silent: false,
                            display_name: "start".to_string(),
                        },
                        NamedField {
                            value: Link::Placeholder,
                            doc: "".to_string(),
                            silent: false,
                            display_name: "end".to_string(),
                        },
                    ],
                },
                serde: ContainerSerdeMetadata {
                    name: "Range".to_string(),
                    fields_or_variants: vec![
                        FieldOrVariantSerdeMetadata {
                            name: "start".to_string(),
                        },
                        FieldOrVariantSerdeMetadata {
                            name: "end".to_string(),
                        },
                    ],
                },
            }))
        }

        fn get_child_links(schema: &mut Schema) -> Vec<Link> {
            vec![T::make_linkable(schema), T::make_linkable(schema)]
        }
    }

    #[cfg(feature = "arrayvec")]
    impl<T: UniversalWallet, const N: usize> OverrideSchema for arrayvec::ArrayVec<T, N> {
        type Output = Vec<T>;
    }
}
