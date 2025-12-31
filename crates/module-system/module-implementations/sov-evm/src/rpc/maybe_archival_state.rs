use derive_more::From;
use sov_modules_api::{ApiStateAccessor, Spec};
use std::ops::{Deref, DerefMut};

#[derive(From)]
pub(crate) enum MaybeArchivalState<'a, S: Spec> {
    Current(&'a mut ApiStateAccessor<S>),
    Archival(Box<ApiStateAccessor<S>>),
}

impl<'a, S: Spec> Deref for MaybeArchivalState<'a, S> {
    type Target = ApiStateAccessor<S>;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Current(a) => a,
            Self::Archival(a) => a,
        }
    }
}

impl<'a, S: Spec> DerefMut for MaybeArchivalState<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Current(a) => a,
            Self::Archival(a) => a,
        }
    }
}
