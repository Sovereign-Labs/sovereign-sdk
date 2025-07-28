mod basic;

mod soft_confirmations;

pub type S = sov_test_utils::TestSpec;

pub type TestRunner<RT> = sov_test_utils::runtime::TestRunner<RT, S>;
