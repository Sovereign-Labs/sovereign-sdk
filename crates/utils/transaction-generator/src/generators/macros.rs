//! A macro that implements [`crate::HarnessModule`] for a given harness module.

/// Generates a harness module implementation for a given module harness struct.
#[macro_export]
macro_rules! impl_harness_module {
    ($harness_name:ident <= generator: $generator_ty:ty) => {
        use $crate::interface::traits::CallMessageGenerator as _;

        /// A wrapper around [`$generator_ty`] that implements [`crate::HarnessModule`].
        #[derive(Debug, Clone)]
        pub struct $harness_name<S: ::sov_modules_api::Spec, RT: ::sov_modules_api::DispatchCall, Tag, ChangelogEntry, BonusAcctData>(
            $generator_ty,
            std::marker::PhantomData<(RT, Tag, ChangelogEntry, BonusAcctData)>,
        );

        impl<S: ::sov_modules_api::Spec, RT: ::sov_modules_api::DispatchCall, Tag, ChangelogEntry, BonusAcctData>
            $harness_name<S, RT, Tag, ChangelogEntry, BonusAcctData>
        {
            /// Create a new `$harness_name` from a `$generator_ty`
            pub fn new(message_generator: $generator_ty) -> Self {
                $harness_name(message_generator, Default::default())
            }

            /// Returns a reference to the inner [`$generator_ty`]
            pub fn inner(&self) -> &$generator_ty {
                &self.0
            }
        }

        impl<
                S: ::sov_modules_api::Spec,
                RT: ::sov_modules_api::EncodeCall<
                    <$generator_ty as $crate::interface::traits::CallMessageGenerator<S>>::Module,
                >,
                Tag: Eq
                    + ::std::hash::Hash
                    + Clone
                    + ::std::fmt::Debug
                    + From<<$generator_ty as $crate::interface::traits::CallMessageGenerator<S>>::Tag>
                    + Send
                    + Sync,
                ChangelogEntry: $crate::ChangelogEntry +
                From<<$generator_ty as $crate::interface::traits::CallMessageGenerator<S>>::ChangelogEntry>
                    + Send
                    + Sync,
                BonusAcctData: Default + Clone + 'static + Sync + Send,
            > $crate::HarnessModule<S, RT, Tag, ChangelogEntry, BonusAcctData>
            for $harness_name<S, RT, Tag, ChangelogEntry, BonusAcctData>
        {
            fn generate_setup_messages(
                &self,
                u: &mut ::sov_modules_api::prelude::arbitrary::Unstructured<'_>,
                generator_state: &mut $crate::State<S, Tag, BonusAcctData>,
            ) -> ::sov_modules_api::prelude::arbitrary::Result<
                Vec<
                    $crate::GeneratedMessage<
                        S,
                        <RT as ::sov_modules_api::DispatchCall>::Decodable,
                        ChangelogEntry,
                    >,
                >,
            > {
                Ok(self
                    .0
                    .generate_setup_messages(
                        u,
                        &mut $crate::GeneratorStateMapper::<_, _, Tag, _>::new(generator_state),
                    )?
                    .into_iter()
                    .map(|m| $crate::GeneratedMessage {
                        message: <RT as ::sov_modules_api::EncodeCall<
                    <$generator_ty as $crate::interface::traits::CallMessageGenerator<S>>::Module>>::to_decodable(
                            m.message,
                        ),
                        sender: m.sender,
                        outcome: m.outcome.map(Into::into),
                    })
                    .collect())
            }

            fn generate_call_message(
                &self,
                u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
                generator_state: &mut $crate::State<S, Tag, BonusAcctData>,
                validity: $crate::MessageValidity,
            ) -> ::sov_modules_api::prelude::arbitrary::Result<
                $crate::GeneratedMessage<
                    S,
                    <RT as ::sov_modules_api::DispatchCall>::Decodable,
                    ChangelogEntry,
                >,
            > {
                self.0
                    .generate_call_message(
                        u,
                        &mut $crate::GeneratorStateMapper::<_, _, Tag, _>::new(generator_state),
                        validity,
                    )
                    .map(|m| $crate::GeneratedMessage {
                        message: <RT as ::sov_modules_api::EncodeCall<<$generator_ty as $crate::interface::traits::CallMessageGenerator<S>>::Module>>::to_decodable(
                            m.message,
                        ),
                        sender: m.sender,
                        outcome: m.outcome.map(Into::into),
                    })
            }
        }
    };
}
