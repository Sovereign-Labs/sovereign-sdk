use sov_modules_api::Prefix;

pub(crate) trait Context: Clone {
    type Storage: Clone;
}

#[derive(Clone)]
pub(crate) struct TestStorage {}

#[derive(Clone)]
pub(crate) struct TestContext {}

impl Context for TestContext {
    type Storage = TestStorage;
}

#[allow(dead_code)]
pub(crate) struct TestState<S> {
    _storage: S,
    pub(crate) prefix: Prefix,
}

impl<S> TestState<S> {
    #[allow(dead_code)]
    pub fn new(_storage: S, prefix: Prefix) -> Self {
        Self { _storage, prefix }
    }
}
