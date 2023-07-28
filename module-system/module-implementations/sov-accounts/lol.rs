mod query {
    #![allow(missing_docs)]
    use jsonrpsee::core::RpcResult;
    use sov_modules_api::macros::rpc_gen;
    use sov_modules_api::AddressBech32;
    use sov_state::WorkingSet;

    use crate::{Account, Accounts};
    /// This is the response returned from the accounts_getAccount endpoint.
    pub enum Response {
        /// The account corresponding to the given public key exists.
        AccountExists {
            /// The address of the account,
            addr: AddressBech32,
            /// The nonce of the account.
            nonce: u64,
        },
        /// The account corresponding to the given public key does not exist.
        AccountEmpty,
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl<'de> _serde::Deserialize<'de> for Response {
            fn deserialize<__D>(__deserializer: __D) -> _serde::__private::Result<Self, __D::Error>
            where
                __D: _serde::Deserializer<'de>,
            {
                #[allow(non_camel_case_types)]
                #[doc(hidden)]
                enum __Field {
                    __field0,
                    __field1,
                }
                #[doc(hidden)]
                struct __FieldVisitor;
                impl<'de> _serde::de::Visitor<'de> for __FieldVisitor {
                    type Value = __Field;
                    fn expecting(
                        &self,
                        __formatter: &mut _serde::__private::Formatter,
                    ) -> _serde::__private::fmt::Result {
                        _serde::__private::Formatter::write_str(__formatter, "variant identifier")
                    }
                    fn visit_u64<__E>(
                        self,
                        __value: u64,
                    ) -> _serde::__private::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            0u64 => _serde::__private::Ok(__Field::__field0),
                            1u64 => _serde::__private::Ok(__Field::__field1),
                            _ => _serde::__private::Err(_serde::de::Error::invalid_value(
                                _serde::de::Unexpected::Unsigned(__value),
                                &"variant index 0 <= i < 2",
                            )),
                        }
                    }
                    fn visit_str<__E>(
                        self,
                        __value: &str,
                    ) -> _serde::__private::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            "AccountExists" => _serde::__private::Ok(__Field::__field0),
                            "AccountEmpty" => _serde::__private::Ok(__Field::__field1),
                            _ => _serde::__private::Err(_serde::de::Error::unknown_variant(
                                __value, VARIANTS,
                            )),
                        }
                    }
                    fn visit_bytes<__E>(
                        self,
                        __value: &[u8],
                    ) -> _serde::__private::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            b"AccountExists" => _serde::__private::Ok(__Field::__field0),
                            b"AccountEmpty" => _serde::__private::Ok(__Field::__field1),
                            _ => {
                                let __value = &_serde::__private::from_utf8_lossy(__value);
                                _serde::__private::Err(_serde::de::Error::unknown_variant(
                                    __value, VARIANTS,
                                ))
                            }
                        }
                    }
                }
                impl<'de> _serde::Deserialize<'de> for __Field {
                    #[inline]
                    fn deserialize<__D>(
                        __deserializer: __D,
                    ) -> _serde::__private::Result<Self, __D::Error>
                    where
                        __D: _serde::Deserializer<'de>,
                    {
                        _serde::Deserializer::deserialize_identifier(__deserializer, __FieldVisitor)
                    }
                }
                #[doc(hidden)]
                struct __Visitor<'de> {
                    marker: _serde::__private::PhantomData<Response>,
                    lifetime: _serde::__private::PhantomData<&'de ()>,
                }
                impl<'de> _serde::de::Visitor<'de> for __Visitor<'de> {
                    type Value = Response;
                    fn expecting(
                        &self,
                        __formatter: &mut _serde::__private::Formatter,
                    ) -> _serde::__private::fmt::Result {
                        _serde::__private::Formatter::write_str(__formatter, "enum Response")
                    }
                    fn visit_enum<__A>(
                        self,
                        __data: __A,
                    ) -> _serde::__private::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::EnumAccess<'de>,
                    {
                        match match _serde::de::EnumAccess::variant(__data) {
                            _serde::__private::Ok(__val) => __val,
                            _serde::__private::Err(__err) => {
                                return _serde::__private::Err(__err);
                            }
                        } {
                            (__Field::__field0, __variant) => {
                                #[allow(non_camel_case_types)]
                                #[doc(hidden)]
                                enum __Field {
                                    __field0,
                                    __field1,
                                    __ignore,
                                }
                                #[doc(hidden)]
                                struct __FieldVisitor;
                                impl<'de> _serde::de::Visitor<'de> for __FieldVisitor {
                                    type Value = __Field;
                                    fn expecting(
                                        &self,
                                        __formatter: &mut _serde::__private::Formatter,
                                    ) -> _serde::__private::fmt::Result
                                    {
                                        _serde::__private::Formatter::write_str(
                                            __formatter,
                                            "field identifier",
                                        )
                                    }
                                    fn visit_u64<__E>(
                                        self,
                                        __value: u64,
                                    ) -> _serde::__private::Result<Self::Value, __E>
                                    where
                                        __E: _serde::de::Error,
                                    {
                                        match __value {
                                            0u64 => _serde::__private::Ok(__Field::__field0),
                                            1u64 => _serde::__private::Ok(__Field::__field1),
                                            _ => _serde::__private::Ok(__Field::__ignore),
                                        }
                                    }
                                    fn visit_str<__E>(
                                        self,
                                        __value: &str,
                                    ) -> _serde::__private::Result<Self::Value, __E>
                                    where
                                        __E: _serde::de::Error,
                                    {
                                        match __value {
                                            "addr" => _serde::__private::Ok(__Field::__field0),
                                            "nonce" => _serde::__private::Ok(__Field::__field1),
                                            _ => _serde::__private::Ok(__Field::__ignore),
                                        }
                                    }
                                    fn visit_bytes<__E>(
                                        self,
                                        __value: &[u8],
                                    ) -> _serde::__private::Result<Self::Value, __E>
                                    where
                                        __E: _serde::de::Error,
                                    {
                                        match __value {
                                            b"addr" => _serde::__private::Ok(__Field::__field0),
                                            b"nonce" => _serde::__private::Ok(__Field::__field1),
                                            _ => _serde::__private::Ok(__Field::__ignore),
                                        }
                                    }
                                }
                                impl<'de> _serde::Deserialize<'de> for __Field {
                                    #[inline]
                                    fn deserialize<__D>(
                                        __deserializer: __D,
                                    ) -> _serde::__private::Result<Self, __D::Error>
                                    where
                                        __D: _serde::Deserializer<'de>,
                                    {
                                        _serde::Deserializer::deserialize_identifier(
                                            __deserializer,
                                            __FieldVisitor,
                                        )
                                    }
                                }
                                #[doc(hidden)]
                                struct __Visitor<'de> {
                                    marker: _serde::__private::PhantomData<Response>,
                                    lifetime: _serde::__private::PhantomData<&'de ()>,
                                }
                                impl<'de> _serde::de::Visitor<'de> for __Visitor<'de> {
                                    type Value = Response;
                                    fn expecting(
                                        &self,
                                        __formatter: &mut _serde::__private::Formatter,
                                    ) -> _serde::__private::fmt::Result
                                    {
                                        _serde::__private::Formatter::write_str(
                                            __formatter,
                                            "struct variant Response::AccountExists",
                                        )
                                    }
                                    #[inline]
                                    fn visit_seq<__A>(
                                        self,
                                        mut __seq: __A,
                                    ) -> _serde::__private::Result<Self::Value, __A::Error>
                                    where
                                        __A: _serde::de::SeqAccess<'de>,
                                    {
                                        let __field0 =
                                            match match _serde::de::SeqAccess::next_element::<
                                                AddressBech32,
                                            >(
                                                &mut __seq
                                            ) {
                                                _serde::__private::Ok(__val) => __val,
                                                _serde::__private::Err(__err) => {
                                                    return _serde::__private::Err(__err);
                                                }
                                            } {
                                                _serde::__private::Some(__value) => __value,
                                                _serde::__private::None => {
                                                    return _serde::__private::Err(
                                                    _serde::de::Error::invalid_length(
                                                        0usize,
                                                        &"struct variant Response::AccountExists with 2 elements",
                                                    ),
                                                );
                                                }
                                            };
                                        let __field1 =
                                            match match _serde::de::SeqAccess::next_element::<u64>(
                                                &mut __seq,
                                            ) {
                                                _serde::__private::Ok(__val) => __val,
                                                _serde::__private::Err(__err) => {
                                                    return _serde::__private::Err(__err);
                                                }
                                            } {
                                                _serde::__private::Some(__value) => __value,
                                                _serde::__private::None => {
                                                    return _serde::__private::Err(
                                                    _serde::de::Error::invalid_length(
                                                        1usize,
                                                        &"struct variant Response::AccountExists with 2 elements",
                                                    ),
                                                );
                                                }
                                            };
                                        _serde::__private::Ok(Response::AccountExists {
                                            addr: __field0,
                                            nonce: __field1,
                                        })
                                    }
                                    #[inline]
                                    fn visit_map<__A>(
                                        self,
                                        mut __map: __A,
                                    ) -> _serde::__private::Result<Self::Value, __A::Error>
                                    where
                                        __A: _serde::de::MapAccess<'de>,
                                    {
                                        let mut __field0: _serde::__private::Option<AddressBech32> =
                                            _serde::__private::None;
                                        let mut __field1: _serde::__private::Option<u64> =
                                            _serde::__private::None;
                                        while let _serde::__private::Some(__key) =
                                            match _serde::de::MapAccess::next_key::<__Field>(
                                                &mut __map,
                                            ) {
                                                _serde::__private::Ok(__val) => __val,
                                                _serde::__private::Err(__err) => {
                                                    return _serde::__private::Err(__err);
                                                }
                                            }
                                        {
                                            match __key {
                                                __Field::__field0 => {
                                                    if _serde::__private::Option::is_some(&__field0)
                                                    {
                                                        return _serde::__private::Err(
                                                            <__A::Error as _serde::de::Error>::duplicate_field("addr"),
                                                        );
                                                    }
                                                    __field0 = _serde::__private::Some(
                                                        match _serde::de::MapAccess::next_value::<
                                                            AddressBech32,
                                                        >(
                                                            &mut __map
                                                        ) {
                                                            _serde::__private::Ok(__val) => __val,
                                                            _serde::__private::Err(__err) => {
                                                                return _serde::__private::Err(
                                                                    __err,
                                                                );
                                                            }
                                                        },
                                                    );
                                                }
                                                __Field::__field1 => {
                                                    if _serde::__private::Option::is_some(&__field1)
                                                    {
                                                        return _serde::__private::Err(
                                                            <__A::Error as _serde::de::Error>::duplicate_field("nonce"),
                                                        );
                                                    }
                                                    __field1 = _serde::__private::Some(
                                                        match _serde::de::MapAccess::next_value::<u64>(
                                                            &mut __map,
                                                        ) {
                                                            _serde::__private::Ok(__val) => __val,
                                                            _serde::__private::Err(__err) => {
                                                                return _serde::__private::Err(
                                                                    __err,
                                                                );
                                                            }
                                                        },
                                                    );
                                                }
                                                _ => {
                                                    let _ = match _serde::de::MapAccess::next_value::<
                                                        _serde::de::IgnoredAny,
                                                    >(
                                                        &mut __map
                                                    ) {
                                                        _serde::__private::Ok(__val) => __val,
                                                        _serde::__private::Err(__err) => {
                                                            return _serde::__private::Err(__err);
                                                        }
                                                    };
                                                }
                                            }
                                        }
                                        let __field0 = match __field0 {
                                            _serde::__private::Some(__field0) => __field0,
                                            _serde::__private::None => {
                                                match _serde::__private::de::missing_field("addr") {
                                                    _serde::__private::Ok(__val) => __val,
                                                    _serde::__private::Err(__err) => {
                                                        return _serde::__private::Err(__err);
                                                    }
                                                }
                                            }
                                        };
                                        let __field1 = match __field1 {
                                            _serde::__private::Some(__field1) => __field1,
                                            _serde::__private::None => {
                                                match _serde::__private::de::missing_field("nonce")
                                                {
                                                    _serde::__private::Ok(__val) => __val,
                                                    _serde::__private::Err(__err) => {
                                                        return _serde::__private::Err(__err);
                                                    }
                                                }
                                            }
                                        };
                                        _serde::__private::Ok(Response::AccountExists {
                                            addr: __field0,
                                            nonce: __field1,
                                        })
                                    }
                                }
                                #[doc(hidden)]
                                const FIELDS: &'static [&'static str] = &["addr", "nonce"];
                                _serde::de::VariantAccess::struct_variant(
                                    __variant,
                                    FIELDS,
                                    __Visitor {
                                        marker: _serde::__private::PhantomData::<Response>,
                                        lifetime: _serde::__private::PhantomData,
                                    },
                                )
                            }
                            (__Field::__field1, __variant) => {
                                match _serde::de::VariantAccess::unit_variant(__variant) {
                                    _serde::__private::Ok(__val) => __val,
                                    _serde::__private::Err(__err) => {
                                        return _serde::__private::Err(__err);
                                    }
                                };
                                _serde::__private::Ok(Response::AccountEmpty)
                            }
                        }
                    }
                }
                #[doc(hidden)]
                const VARIANTS: &'static [&'static str] = &["AccountExists", "AccountEmpty"];
                _serde::Deserializer::deserialize_enum(
                    __deserializer,
                    "Response",
                    VARIANTS,
                    __Visitor {
                        marker: _serde::__private::PhantomData::<Response>,
                        lifetime: _serde::__private::PhantomData,
                    },
                )
            }
        }
    };
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(unused_extern_crates, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for Response {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::__private::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    Response::AccountExists {
                        ref addr,
                        ref nonce,
                    } => {
                        let mut __serde_state = match _serde::Serializer::serialize_struct_variant(
                            __serializer,
                            "Response",
                            0u32,
                            "AccountExists",
                            0 + 1 + 1,
                        ) {
                            _serde::__private::Ok(__val) => __val,
                            _serde::__private::Err(__err) => {
                                return _serde::__private::Err(__err);
                            }
                        };
                        match _serde::ser::SerializeStructVariant::serialize_field(
                            &mut __serde_state,
                            "addr",
                            addr,
                        ) {
                            _serde::__private::Ok(__val) => __val,
                            _serde::__private::Err(__err) => {
                                return _serde::__private::Err(__err);
                            }
                        };
                        match _serde::ser::SerializeStructVariant::serialize_field(
                            &mut __serde_state,
                            "nonce",
                            nonce,
                        ) {
                            _serde::__private::Ok(__val) => __val,
                            _serde::__private::Err(__err) => {
                                return _serde::__private::Err(__err);
                            }
                        };
                        _serde::ser::SerializeStructVariant::end(__serde_state)
                    }
                    Response::AccountEmpty => _serde::Serializer::serialize_unit_variant(
                        __serializer,
                        "Response",
                        1u32,
                        "AccountEmpty",
                    ),
                }
            }
        }
    };
    impl<C: sov_modules_api::Context> Accounts<C> {
        pub fn get_account(
            &self,
            pub_key: C::PublicKey,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> RpcResult<Response> {
            let response = match self.accounts.get(&pub_key, working_set) {
                Some(Account { addr, nonce }) => Response::AccountExists {
                    addr: addr.into(),
                    nonce,
                },
                None => Response::AccountEmpty,
            };
            Ok(response)
        }
    }
    pub trait AccountsRpcImpl<C: sov_modules_api::Context> {
        fn get_working_set(&self) -> WorkingSet<C::Storage>;
        fn get_account(&self, pub_key: C::PublicKey) -> RpcResult<Response> {
            <Accounts<C> as ::std::default::Default>::default()
                .get_account(pub_key, &mut Self::get_working_set(self))
        }
    }
    impl<
            MacroGeneratedTypeWithLongNameToAvoidCollisions: AccountsRpcImpl<C> + Send + Sync + 'static,
            C: sov_modules_api::Context,
        > AccountsRpcServer<C> for MacroGeneratedTypeWithLongNameToAvoidCollisions
    {
        fn get_account(&self, pub_key: C::PublicKey) -> RpcResult<Response> {
            Ok(<Self as AccountsRpcImpl<C>>::get_account(self, pub_key))
        }
    }
    ///Server trait implementation for the `AccountsRpc` RPC API.
    pub trait AccountsRpcServer<C: sov_modules_api::Context>:
        Sized + Send + Sync + 'static
    {
        fn get_account(&self, pub_key: C::PublicKey) -> RpcResult<Response>;
        fn health(&self) -> ::jsonrpsee::core::RpcResult<()> {
            Ok(())
        }
        ///Collects all the methods and subscriptions defined in the trait and adds them into a single `RpcModule`.
        fn into_rpc(self) -> jsonrpsee::RpcModule<Self> {
            let mut rpc = jsonrpsee::RpcModule::new(self);
            {
                let res = rpc
                    .register_method(
                        "accounts_getAccount",
                        |params, context| {
                            let (pub_key) = if params.is_object() {
                                #[serde(
                                    crate = "jsonrpsee :: core :: __reexports :: serde"
                                )]
                                struct ParamsObject<G0> {
                                    #[serde(alias = "pub_key", alias = "pubKey")]
                                    pub_key: G0,
                                }
                                #[doc(hidden)]
                                #[allow(
                                    non_upper_case_globals,
                                    unused_attributes,
                                    unused_qualifications
                                )]
                                const _: () = {
                                    use jsonrpsee::core::__reexports::serde as _serde;
                                    #[automatically_derived]
                                    impl<
                                        'de,
                                        G0,
                                    > jsonrpsee::core::__reexports::serde::Deserialize<'de>
                                    for ParamsObject<G0>
                                    where
                                        G0: _serde::Deserialize<'de>,
                                    {
                                        fn deserialize<__D>(
                                            __deserializer: __D,
                                        ) -> jsonrpsee::core::__reexports::serde::__private::Result<
                                            Self,
                                            __D::Error,
                                        >
                                        where
                                            __D: jsonrpsee::core::__reexports::serde::Deserializer<'de>,
                                        {
                                            #[allow(non_camel_case_types)]
                                            #[doc(hidden)]
                                            enum __Field {
                                                __field0,
                                                __ignore,
                                            }
                                            #[doc(hidden)]
                                            struct __FieldVisitor;
                                            impl<'de> _serde::de::Visitor<'de> for __FieldVisitor {
                                                type Value = __Field;
                                                fn expecting(
                                                    &self,
                                                    __formatter: &mut _serde::__private::Formatter,
                                                ) -> _serde::__private::fmt::Result {
                                                    _serde::__private::Formatter::write_str(
                                                        __formatter,
                                                        "field identifier",
                                                    )
                                                }
                                                fn visit_u64<__E>(
                                                    self,
                                                    __value: u64,
                                                ) -> _serde::__private::Result<Self::Value, __E>
                                                where
                                                    __E: _serde::de::Error,
                                                {
                                                    match __value {
                                                        0u64 => _serde::__private::Ok(__Field::__field0),
                                                        _ => _serde::__private::Ok(__Field::__ignore),
                                                    }
                                                }
                                                fn visit_str<__E>(
                                                    self,
                                                    __value: &str,
                                                ) -> _serde::__private::Result<Self::Value, __E>
                                                where
                                                    __E: _serde::de::Error,
                                                {
                                                    match __value {
                                                        "pubKey" => _serde::__private::Ok(__Field::__field0),
                                                        "pub_key" => _serde::__private::Ok(__Field::__field0),
                                                        _ => _serde::__private::Ok(__Field::__ignore),
                                                    }
                                                }
                                                fn visit_bytes<__E>(
                                                    self,
                                                    __value: &[u8],
                                                ) -> _serde::__private::Result<Self::Value, __E>
                                                where
                                                    __E: _serde::de::Error,
                                                {
                                                    match __value {
                                                        b"pubKey" => _serde::__private::Ok(__Field::__field0),
                                                        b"pub_key" => _serde::__private::Ok(__Field::__field0),
                                                        _ => _serde::__private::Ok(__Field::__ignore),
                                                    }
                                                }
                                            }
                                            impl<'de> _serde::Deserialize<'de> for __Field {
                                                #[inline]
                                                fn deserialize<__D>(
                                                    __deserializer: __D,
                                                ) -> _serde::__private::Result<Self, __D::Error>
                                                where
                                                    __D: _serde::Deserializer<'de>,
                                                {
                                                    _serde::Deserializer::deserialize_identifier(
                                                        __deserializer,
                                                        __FieldVisitor,
                                                    )
                                                }
                                            }
                                            #[doc(hidden)]
                                            struct __Visitor<'de, G0>
                                            where
                                                G0: _serde::Deserialize<'de>,
                                            {
                                                marker: _serde::__private::PhantomData<ParamsObject<G0>>,
                                                lifetime: _serde::__private::PhantomData<&'de ()>,
                                            }
                                            impl<'de, G0> _serde::de::Visitor<'de>
                                            for __Visitor<'de, G0>
                                            where
                                                G0: _serde::Deserialize<'de>,
                                            {
                                                type Value = ParamsObject<G0>;
                                                fn expecting(
                                                    &self,
                                                    __formatter: &mut _serde::__private::Formatter,
                                                ) -> _serde::__private::fmt::Result {
                                                    _serde::__private::Formatter::write_str(
                                                        __formatter,
                                                        "struct ParamsObject",
                                                    )
                                                }
                                                #[inline]
                                                fn visit_seq<__A>(
                                                    self,
                                                    mut __seq: __A,
                                                ) -> _serde::__private::Result<Self::Value, __A::Error>
                                                where
                                                    __A: _serde::de::SeqAccess<'de>,
                                                {
                                                    let __field0 = match match _serde::de::SeqAccess::next_element::<
                                                        G0,
                                                    >(&mut __seq) {
                                                        _serde::__private::Ok(__val) => __val,
                                                        _serde::__private::Err(__err) => {
                                                            return _serde::__private::Err(__err);
                                                        }
                                                    } {
                                                        _serde::__private::Some(__value) => __value,
                                                        _serde::__private::None => {
                                                            return _serde::__private::Err(
                                                                _serde::de::Error::invalid_length(
                                                                    0usize,
                                                                    &"struct ParamsObject with 1 element",
                                                                ),
                                                            );
                                                        }
                                                    };
                                                    _serde::__private::Ok(ParamsObject { pub_key: __field0 })
                                                }
                                                #[inline]
                                                fn visit_map<__A>(
                                                    self,
                                                    mut __map: __A,
                                                ) -> _serde::__private::Result<Self::Value, __A::Error>
                                                where
                                                    __A: _serde::de::MapAccess<'de>,
                                                {
                                                    let mut __field0: _serde::__private::Option<G0> = _serde::__private::None;
                                                    while let _serde::__private::Some(__key)
                                                        = match _serde::de::MapAccess::next_key::<
                                                            __Field,
                                                        >(&mut __map) {
                                                            _serde::__private::Ok(__val) => __val,
                                                            _serde::__private::Err(__err) => {
                                                                return _serde::__private::Err(__err);
                                                            }
                                                        } {
                                                        match __key {
                                                            __Field::__field0 => {
                                                                if _serde::__private::Option::is_some(&__field0) {
                                                                    return _serde::__private::Err(
                                                                        <__A::Error as _serde::de::Error>::duplicate_field(
                                                                            "pub_key",
                                                                        ),
                                                                    );
                                                                }
                                                                __field0 = _serde::__private::Some(
                                                                    match _serde::de::MapAccess::next_value::<G0>(&mut __map) {
                                                                        _serde::__private::Ok(__val) => __val,
                                                                        _serde::__private::Err(__err) => {
                                                                            return _serde::__private::Err(__err);
                                                                        }
                                                                    },
                                                                );
                                                            }
                                                            _ => {
                                                                let _ = match _serde::de::MapAccess::next_value::<
                                                                    _serde::de::IgnoredAny,
                                                                >(&mut __map) {
                                                                    _serde::__private::Ok(__val) => __val,
                                                                    _serde::__private::Err(__err) => {
                                                                        return _serde::__private::Err(__err);
                                                                    }
                                                                };
                                                            }
                                                        }
                                                    }
                                                    let __field0 = match __field0 {
                                                        _serde::__private::Some(__field0) => __field0,
                                                        _serde::__private::None => {
                                                            match _serde::__private::de::missing_field("pub_key") {
                                                                _serde::__private::Ok(__val) => __val,
                                                                _serde::__private::Err(__err) => {
                                                                    return _serde::__private::Err(__err);
                                                                }
                                                            }
                                                        }
                                                    };
                                                    _serde::__private::Ok(ParamsObject { pub_key: __field0 })
                                                }
                                            }
                                            #[doc(hidden)]
                                            const FIELDS: &'static [&'static str] = &[
                                                "pubKey",
                                                "pub_key",
                                            ];
                                            _serde::Deserializer::deserialize_struct(
                                                __deserializer,
                                                "ParamsObject",
                                                FIELDS,
                                                __Visitor {
                                                    marker: _serde::__private::PhantomData::<ParamsObject<G0>>,
                                                    lifetime: _serde::__private::PhantomData,
                                                },
                                            )
                                        }
                                    }
                                };
                                let parsed: ParamsObject<C::PublicKey> = params
                                    .parse()
                                    .map_err(|e| {
                                        {
                                            use ::tracing::__macro_support::Callsite as _;
                                            static CALLSITE: ::tracing::callsite::DefaultCallsite = {
                                                static META: ::tracing::Metadata<'static> = {
                                                    ::tracing_core::metadata::Metadata::new(
                                                        "event module-system/module-implementations/sov-accounts/src/query.rs:23",
                                                        "sov_accounts::query",
                                                        ::tracing::Level::ERROR,
                                                        Some(
                                                            "module-system/module-implementations/sov-accounts/src/query.rs",
                                                        ),
                                                        Some(23u32),
                                                        Some("sov_accounts::query"),
                                                        ::tracing_core::field::FieldSet::new(
                                                            &["message"],
                                                            ::tracing_core::callsite::Identifier(&CALLSITE),
                                                        ),
                                                        ::tracing::metadata::Kind::EVENT,
                                                    )
                                                };
                                                ::tracing::callsite::DefaultCallsite::new(&META)
                                            };
                                            let enabled = ::tracing::Level::ERROR
                                                <= ::tracing::level_filters::STATIC_MAX_LEVEL
                                                && ::tracing::Level::ERROR
                                                    <= ::tracing::level_filters::LevelFilter::current()
                                                && {
                                                    let interest = CALLSITE.interest();
                                                    !interest.is_never()
                                                        && ::tracing::__macro_support::__is_enabled(
                                                            CALLSITE.metadata(),
                                                            interest,
                                                        )
                                                };
                                            if enabled {
                                                (|value_set: ::tracing::field::ValueSet| {
                                                    let meta = CALLSITE.metadata();
                                                    ::tracing::Event::dispatch(meta, &value_set);
                                                    if (match ::tracing::Level::ERROR {
                                                        ::tracing::Level::ERROR => ::tracing::log::Level::Error,
                                                        ::tracing::Level::WARN => ::tracing::log::Level::Warn,
                                                        ::tracing::Level::INFO => ::tracing::log::Level::Info,
                                                        ::tracing::Level::DEBUG => ::tracing::log::Level::Debug,
                                                        _ => ::tracing::log::Level::Trace,
                                                    }) <= ::tracing::log::STATIC_MAX_LEVEL
                                                    {
                                                        if !::tracing::dispatcher::has_been_set() {
                                                            {
                                                                use ::tracing::log;
                                                                let level = match ::tracing::Level::ERROR {
                                                                    ::tracing::Level::ERROR => ::tracing::log::Level::Error,
                                                                    ::tracing::Level::WARN => ::tracing::log::Level::Warn,
                                                                    ::tracing::Level::INFO => ::tracing::log::Level::Info,
                                                                    ::tracing::Level::DEBUG => ::tracing::log::Level::Debug,
                                                                    _ => ::tracing::log::Level::Trace,
                                                                };
                                                                if level <= log::max_level() {
                                                                    let meta = CALLSITE.metadata();
                                                                    let log_meta = log::Metadata::builder()
                                                                        .level(level)
                                                                        .target(meta.target())
                                                                        .build();
                                                                    let logger = log::logger();
                                                                    if logger.enabled(&log_meta) {
                                                                        ::tracing::__macro_support::__tracing_log(
                                                                            meta,
                                                                            logger,
                                                                            log_meta,
                                                                            &value_set,
                                                                        )
                                                                    }
                                                                }
                                                            }
                                                        } else {
                                                            {}
                                                        }
                                                    } else {
                                                        {}
                                                    };
                                                })({
                                                    #[allow(unused_imports)]
                                                    use ::tracing::field::{debug, display, Value};
                                                    let mut iter = CALLSITE.metadata().fields().iter();
                                                    CALLSITE
                                                        .metadata()
                                                        .fields()
                                                        .value_set(
                                                            &[
                                                                (
                                                                    &iter.next().expect("FieldSet corrupted (this is a bug)"),
                                                                    Some(
                                                                        &format_args!(
                                                                            "Failed to parse JSON-RPC params as object: {0}", e
                                                                        ) as &dyn Value,
                                                                    ),
                                                                ),
                                                            ],
                                                        )
                                                });
                                            } else {
                                                if (match ::tracing::Level::ERROR {
                                                    ::tracing::Level::ERROR => ::tracing::log::Level::Error,
                                                    ::tracing::Level::WARN => ::tracing::log::Level::Warn,
                                                    ::tracing::Level::INFO => ::tracing::log::Level::Info,
                                                    ::tracing::Level::DEBUG => ::tracing::log::Level::Debug,
                                                    _ => ::tracing::log::Level::Trace,
                                                }) <= ::tracing::log::STATIC_MAX_LEVEL
                                                {
                                                    if !::tracing::dispatcher::has_been_set() {
                                                        {
                                                            use ::tracing::log;
                                                            let level = match ::tracing::Level::ERROR {
                                                                ::tracing::Level::ERROR => ::tracing::log::Level::Error,
                                                                ::tracing::Level::WARN => ::tracing::log::Level::Warn,
                                                                ::tracing::Level::INFO => ::tracing::log::Level::Info,
                                                                ::tracing::Level::DEBUG => ::tracing::log::Level::Debug,
                                                                _ => ::tracing::log::Level::Trace,
                                                            };
                                                            if level <= log::max_level() {
                                                                let meta = CALLSITE.metadata();
                                                                let log_meta = log::Metadata::builder()
                                                                    .level(level)
                                                                    .target(meta.target())
                                                                    .build();
                                                                let logger = log::logger();
                                                                if logger.enabled(&log_meta) {
                                                                    ::tracing::__macro_support::__tracing_log(
                                                                        meta,
                                                                        logger,
                                                                        log_meta,
                                                                        &{
                                                                            #[allow(unused_imports)]
                                                                            use ::tracing::field::{debug, display, Value};
                                                                            let mut iter = CALLSITE.metadata().fields().iter();
                                                                            CALLSITE
                                                                                .metadata()
                                                                                .fields()
                                                                                .value_set(
                                                                                    &[
                                                                                        (
                                                                                            &iter.next().expect("FieldSet corrupted (this is a bug)"),
                                                                                            Some(
                                                                                                &format_args!(
                                                                                                    "Failed to parse JSON-RPC params as object: {0}", e
                                                                                                ) as &dyn Value,
                                                                                            ),
                                                                                        ),
                                                                                    ],
                                                                                )
                                                                        },
                                                                    )
                                                                }
                                                            }
                                                        }
                                                    } else {
                                                        {}
                                                    }
                                                } else {
                                                    {}
                                                };
                                            }
                                        };
                                        e
                                    })?;
                                (parsed.pub_key)
                            } else {
                                let mut seq = params.sequence();
                                let pub_key: C::PublicKey = match seq.next() {
                                    Ok(v) => v,
                                    Err(e) => {
                                        {
                                            use ::tracing::__macro_support::Callsite as _;
                                            static CALLSITE: ::tracing::callsite::DefaultCallsite = {
                                                static META: ::tracing::Metadata<'static> = {
                                                    ::tracing_core::metadata::Metadata::new(
                                                        "event module-system/module-implementations/sov-accounts/src/query.rs:23",
                                                        "sov_accounts::query",
                                                        ::tracing::Level::ERROR,
                                                        Some(
                                                            "module-system/module-implementations/sov-accounts/src/query.rs",
                                                        ),
                                                        Some(23u32),
                                                        Some("sov_accounts::query"),
                                                        ::tracing_core::field::FieldSet::new(
                                                            &["message"],
                                                            ::tracing_core::callsite::Identifier(&CALLSITE),
                                                        ),
                                                        ::tracing::metadata::Kind::EVENT,
                                                    )
                                                };
                                                ::tracing::callsite::DefaultCallsite::new(&META)
                                            };
                                            let enabled = ::tracing::Level::ERROR
                                                <= ::tracing::level_filters::STATIC_MAX_LEVEL
                                                && ::tracing::Level::ERROR
                                                    <= ::tracing::level_filters::LevelFilter::current()
                                                && {
                                                    let interest = CALLSITE.interest();
                                                    !interest.is_never()
                                                        && ::tracing::__macro_support::__is_enabled(
                                                            CALLSITE.metadata(),
                                                            interest,
                                                        )
                                                };
                                            if enabled {
                                                (|value_set: ::tracing::field::ValueSet| {
                                                    let meta = CALLSITE.metadata();
                                                    ::tracing::Event::dispatch(meta, &value_set);
                                                    if (match ::tracing::Level::ERROR {
                                                        ::tracing::Level::ERROR => ::tracing::log::Level::Error,
                                                        ::tracing::Level::WARN => ::tracing::log::Level::Warn,
                                                        ::tracing::Level::INFO => ::tracing::log::Level::Info,
                                                        ::tracing::Level::DEBUG => ::tracing::log::Level::Debug,
                                                        _ => ::tracing::log::Level::Trace,
                                                    }) <= ::tracing::log::STATIC_MAX_LEVEL
                                                    {
                                                        if !::tracing::dispatcher::has_been_set() {
                                                            {
                                                                use ::tracing::log;
                                                                let level = match ::tracing::Level::ERROR {
                                                                    ::tracing::Level::ERROR => ::tracing::log::Level::Error,
                                                                    ::tracing::Level::WARN => ::tracing::log::Level::Warn,
                                                                    ::tracing::Level::INFO => ::tracing::log::Level::Info,
                                                                    ::tracing::Level::DEBUG => ::tracing::log::Level::Debug,
                                                                    _ => ::tracing::log::Level::Trace,
                                                                };
                                                                if level <= log::max_level() {
                                                                    let meta = CALLSITE.metadata();
                                                                    let log_meta = log::Metadata::builder()
                                                                        .level(level)
                                                                        .target(meta.target())
                                                                        .build();
                                                                    let logger = log::logger();
                                                                    if logger.enabled(&log_meta) {
                                                                        ::tracing::__macro_support::__tracing_log(
                                                                            meta,
                                                                            logger,
                                                                            log_meta,
                                                                            &value_set,
                                                                        )
                                                                    }
                                                                }
                                                            }
                                                        } else {
                                                            {}
                                                        }
                                                    } else {
                                                        {}
                                                    };
                                                })({
                                                    #[allow(unused_imports)]
                                                    use ::tracing::field::{debug, display, Value};
                                                    let mut iter = CALLSITE.metadata().fields().iter();
                                                    CALLSITE
                                                        .metadata()
                                                        .fields()
                                                        .value_set(
                                                            &[
                                                                (
                                                                    &iter.next().expect("FieldSet corrupted (this is a bug)"),
                                                                    Some(
                                                                        &format_args!(
                                                                            "Error parsing \"pub_key\" as \"C :: PublicKey\": {0:?}", e
                                                                        ) as &dyn Value,
                                                                    ),
                                                                ),
                                                            ],
                                                        )
                                                });
                                            } else {
                                                if (match ::tracing::Level::ERROR {
                                                    ::tracing::Level::ERROR => ::tracing::log::Level::Error,
                                                    ::tracing::Level::WARN => ::tracing::log::Level::Warn,
                                                    ::tracing::Level::INFO => ::tracing::log::Level::Info,
                                                    ::tracing::Level::DEBUG => ::tracing::log::Level::Debug,
                                                    _ => ::tracing::log::Level::Trace,
                                                }) <= ::tracing::log::STATIC_MAX_LEVEL
                                                {
                                                    if !::tracing::dispatcher::has_been_set() {
                                                        {
                                                            use ::tracing::log;
                                                            let level = match ::tracing::Level::ERROR {
                                                                ::tracing::Level::ERROR => ::tracing::log::Level::Error,
                                                                ::tracing::Level::WARN => ::tracing::log::Level::Warn,
                                                                ::tracing::Level::INFO => ::tracing::log::Level::Info,
                                                                ::tracing::Level::DEBUG => ::tracing::log::Level::Debug,
                                                                _ => ::tracing::log::Level::Trace,
                                                            };
                                                            if level <= log::max_level() {
                                                                let meta = CALLSITE.metadata();
                                                                let log_meta = log::Metadata::builder()
                                                                    .level(level)
                                                                    .target(meta.target())
                                                                    .build();
                                                                let logger = log::logger();
                                                                if logger.enabled(&log_meta) {
                                                                    ::tracing::__macro_support::__tracing_log(
                                                                        meta,
                                                                        logger,
                                                                        log_meta,
                                                                        &{
                                                                            #[allow(unused_imports)]
                                                                            use ::tracing::field::{debug, display, Value};
                                                                            let mut iter = CALLSITE.metadata().fields().iter();
                                                                            CALLSITE
                                                                                .metadata()
                                                                                .fields()
                                                                                .value_set(
                                                                                    &[
                                                                                        (
                                                                                            &iter.next().expect("FieldSet corrupted (this is a bug)"),
                                                                                            Some(
                                                                                                &format_args!(
                                                                                                    "Error parsing \"pub_key\" as \"C :: PublicKey\": {0:?}", e
                                                                                                ) as &dyn Value,
                                                                                            ),
                                                                                        ),
                                                                                    ],
                                                                                )
                                                                        },
                                                                    )
                                                                }
                                                            }
                                                        }
                                                    } else {
                                                        {}
                                                    }
                                                } else {
                                                    {}
                                                };
                                            }
                                        };
                                        return Err(e.into());
                                    }
                                };
                                (pub_key)
                            };
                            context.get_account(pub_key)
                        },
                    );
                if true {
                    if !res.is_ok() {
                        {
                            ::core::panicking::panic_fmt(
                                format_args!(
                                    "RPC macro method names should never conflict, this is a bug, please report it."
                                ),
                            );
                        }
                    }
                }
            }
            {
                let res =
                    rpc.register_method("accounts_health", |params, context| context.health());
                if true {
                    if !res.is_ok() {
                        {
                            ::core::panicking::panic_fmt(
                                format_args!(
                                    "RPC macro method names should never conflict, this is a bug, please report it."
                                ),
                            );
                        }
                    }
                }
            }
            rpc
        }
    }
    ///Client implementation for the `AccountsRpc` RPC API.
    pub trait AccountsRpcClient<C: sov_modules_api::Context>:
        jsonrpsee::core::client::ClientT
    where
        C: Send + Sync + 'static + jsonrpsee::core::Serialize,
    {
        #[must_use]
        #[allow(
            clippy::async_yields_async,
            clippy::diverging_sub_expression,
            clippy::let_unit_value,
            clippy::no_effect_underscore_binding,
            clippy::shadow_same,
            clippy::type_complexity,
            clippy::type_repetition_in_bounds,
            clippy::used_underscore_binding
        )]
        fn get_account<'life0, 'async_trait>(
            &'life0 self,
            pub_key: C::PublicKey,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = RpcResult<Response>>
                    + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: ::core::marker::Sync + 'async_trait,
        {
            Box::pin(async move {
                if let ::core::option::Option::Some(__ret) =
                    ::core::option::Option::None::<RpcResult<Response>>
                {
                    return __ret;
                }
                let __self = self;
                let pub_key = pub_key;
                let __ret: RpcResult<Response> = {
                    let params = {
                        {
                            let mut params = jsonrpsee::core::params::ArrayParams::new();
                            if let Err(err) = params.insert(pub_key) {
                                {
                                    ::core::panicking::panic_fmt(format_args!(
                                        "Parameter `{0}` cannot be serialized: {1:?}",
                                        "pub_key", err
                                    ));
                                };
                            }
                            params
                        }
                    };
                    __self.request("accounts_getAccount", params).await
                };
                #[allow(unreachable_code)]
                __ret
            })
        }
        #[must_use]
        #[allow(
            clippy::async_yields_async,
            clippy::diverging_sub_expression,
            clippy::let_unit_value,
            clippy::no_effect_underscore_binding,
            clippy::shadow_same,
            clippy::type_complexity,
            clippy::type_repetition_in_bounds,
            clippy::used_underscore_binding
        )]
        fn health<'life0, 'async_trait>(
            &'life0 self,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = ::jsonrpsee::core::RpcResult<()>>
                    + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: ::core::marker::Sync + 'async_trait,
        {
            Box::pin(async move {
                if let ::core::option::Option::Some(__ret) =
                    ::core::option::Option::None::<::jsonrpsee::core::RpcResult<()>>
                {
                    return __ret;
                }
                let __self = self;
                let __ret: ::jsonrpsee::core::RpcResult<()> = {
                    let params = {
                        {
                            jsonrpsee::core::params::ArrayParams::new()
                        }
                    };
                    __self.request("accounts_health", params).await
                };
                #[allow(unreachable_code)]
                __ret
            })
        }
    }
    impl<TypeJsonRpseeInteral, C: sov_modules_api::Context> AccountsRpcClient<C>
        for TypeJsonRpseeInteral
    where
        TypeJsonRpseeInteral: jsonrpsee::core::client::ClientT,
        C: Send + Sync + 'static + jsonrpsee::core::Serialize,
    {
    }
}
pub use call::{CallMessage, UPDATE_ACCOUNT_MSG};
#[cfg(feature = "native")]
pub use query::{AccountsRpcImpl, AccountsRpcServer, Response};
use sov_modules_api::{Error, ModuleInfo};
use sov_state::WorkingSet;
/// Initial configuration for sov-accounts module.
pub struct AccountConfig<C: sov_modules_api::Context> {
    /// Public keys to initialize the rollup.
    pub pub_keys: Vec<C::PublicKey>,
}
/// An account on the rollup.
pub struct Account<C: sov_modules_api::Context> {
    /// The address of the account.
    pub addr: C::Address,
    /// The current nonce value associated with the account.
    pub nonce: u64,
}
impl<C: sov_modules_api::Context> borsh::de::BorshDeserialize for Account<C>
where
    C::Address: borsh::BorshDeserialize,
    u64: borsh::BorshDeserialize,
{
    fn deserialize_reader<R: borsh::maybestd::io::Read>(
        reader: &mut R,
    ) -> ::core::result::Result<Self, borsh::maybestd::io::Error> {
        Ok(Self {
            addr: borsh::BorshDeserialize::deserialize_reader(reader)?,
            nonce: borsh::BorshDeserialize::deserialize_reader(reader)?,
        })
    }
}
impl<C: sov_modules_api::Context> borsh::ser::BorshSerialize for Account<C>
where
    C::Address: borsh::ser::BorshSerialize,
    u64: borsh::ser::BorshSerialize,
{
    fn serialize<W: borsh::maybestd::io::Write>(
        &self,
        writer: &mut W,
    ) -> ::core::result::Result<(), borsh::maybestd::io::Error> {
        borsh::BorshSerialize::serialize(&self.addr, writer)?;
        borsh::BorshSerialize::serialize(&self.nonce, writer)?;
        Ok(())
    }
}
#[automatically_derived]
impl<C: ::core::fmt::Debug + sov_modules_api::Context> ::core::fmt::Debug for Account<C>
where
    C::Address: ::core::fmt::Debug,
{
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field2_finish(
            f,
            "Account",
            "addr",
            &self.addr,
            "nonce",
            &&self.nonce,
        )
    }
}
#[automatically_derived]
impl<C: sov_modules_api::Context> ::core::marker::StructuralPartialEq for Account<C> {}
#[automatically_derived]
impl<C: ::core::cmp::PartialEq + sov_modules_api::Context> ::core::cmp::PartialEq for Account<C>
where
    C::Address: ::core::cmp::PartialEq,
{
    #[inline]
    fn eq(&self, other: &Account<C>) -> bool {
        self.addr == other.addr && self.nonce == other.nonce
    }
}
#[automatically_derived]
impl<C: ::core::marker::Copy + sov_modules_api::Context> ::core::marker::Copy for Account<C> where
    C::Address: ::core::marker::Copy
{
}
#[automatically_derived]
impl<C: ::core::clone::Clone + sov_modules_api::Context> ::core::clone::Clone for Account<C>
where
    C::Address: ::core::clone::Clone,
{
    #[inline]
    fn clone(&self) -> Account<C> {
        Account {
            addr: ::core::clone::Clone::clone(&self.addr),
            nonce: ::core::clone::Clone::clone(&self.nonce),
        }
    }
}
/// A module responsible for managing accounts on the rollup.
pub struct Accounts<C: sov_modules_api::Context> {
    /// The address of the sov-accounts module.
    #[address]
    pub address: C::Address,
    /// Mapping from an account address to a corresponding public key.
    #[state]
    pub(crate) public_keys: sov_state::StateMap<C::Address, C::PublicKey>,
    /// Mapping from a public key to a corresponding account.
    #[state]
    pub(crate) accounts: sov_state::StateMap<C::PublicKey, Account<C>>,
}
impl<C: sov_modules_api::Context> Accounts<C> {
    fn _prefix_public_keys() -> sov_modules_api::Prefix {
        let module_path = "sov_accounts";
        sov_modules_api::Prefix::new_storage(module_path, "Accounts", "public_keys")
    }
    fn _prefix_accounts() -> sov_modules_api::Prefix {
        let module_path = "sov_accounts";
        sov_modules_api::Prefix::new_storage(module_path, "Accounts", "accounts")
    }
}
use ::sov_modules_api::AddressTrait;
impl<C: sov_modules_api::Context> ::std::default::Default for Accounts<C> {
    fn default() -> Self {
        use sov_modules_api::Hasher;
        let module_path = "sov_accounts";
        let prefix = sov_modules_api::Prefix::new_module(module_path, "Accounts");
        let address: <C as sov_modules_api::Spec>::Address =
            <C as ::sov_modules_api::Spec>::Address::try_from(&prefix.hash::<C>()).unwrap_or_else(
                |e| {
                    ::core::panicking::panic_fmt(format_args!(
                        "ModuleInfo macro error, unable to create an Address for module: {0}",
                        e
                    ));
                },
            );
        let state_prefix = Self::_prefix_public_keys().into();
        let public_keys = sov_state::StateMap::new(state_prefix);
        let state_prefix = Self::_prefix_accounts().into();
        let accounts = sov_state::StateMap::new(state_prefix);
        Self {
            address,
            public_keys,
            accounts,
        }
    }
}
impl<C: sov_modules_api::Context> ::sov_modules_api::ModuleInfo for Accounts<C> {
    type Context = C;
    fn address(&self) -> &<Self::Context as sov_modules_api::Spec>::Address {
        &self.address
    }
    fn dependencies(&self) -> ::std::vec::Vec<&<Self::Context as sov_modules_api::Spec>::Address> {
        ::alloc::vec::Vec::new()
    }
}
#[automatically_derived]
impl<C: ::core::clone::Clone + sov_modules_api::Context> ::core::clone::Clone for Accounts<C>
where
    C::Address: ::core::clone::Clone,
    C::Address: ::core::clone::Clone,
    C::PublicKey: ::core::clone::Clone,
    C::PublicKey: ::core::clone::Clone,
{
    #[inline]
    fn clone(&self) -> Accounts<C> {
        Accounts {
            address: ::core::clone::Clone::clone(&self.address),
            public_keys: ::core::clone::Clone::clone(&self.public_keys),
            accounts: ::core::clone::Clone::clone(&self.accounts),
        }
    }
}
use ::schemars::JsonSchema;
impl<C: sov_modules_api::Context> ::sov_modules_api::ModuleCallJsonSchema for Accounts<C> {
    fn json_schema() -> ::std::string::String {
        let schema = ::schemars::gen::SchemaGenerator::default()
            .into_root_schema_for::<<Self as ::sov_modules_api::Module>::CallMessage>();
        ::serde_json::to_string_pretty(&schema)
            .expect("Failed to serialize JSON schema; this is a bug in the module")
    }
}
impl<C: sov_modules_api::Context> sov_modules_api::Module for Accounts<C> {
    type Context = C;
    type Config = AccountConfig<C>;
    type CallMessage = call::CallMessage<C>;
    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }
    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::UpdatePublicKey(new_pub_key, sig) => {
                Ok(self.update_public_key(new_pub_key, sig, context, working_set)?)
            }
        }
    }
}
