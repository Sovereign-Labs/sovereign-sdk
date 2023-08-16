use std::path::Path;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Address;
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::mocks::MockValidityCond;
use sov_state::ProverStorage;

use crate::runtime::Runtime;

mod da_simulation;
mod stf_tests;
mod tx_revert_tests;
pub(crate) type C = DefaultContext;

pub type TestBlob = sov_rollup_interface::mocks::MockBlob<Address>;

pub fn create_new_demo(
    path: impl AsRef<Path>,
) -> AppTemplate<
    DefaultContext,
    MockValidityCond,
    sov_rollup_interface::mocks::MockZkvm,
    Runtime<DefaultContext>,
    TestBlob,
> {
    let runtime = Runtime::default();
    let storage = ProverStorage::with_path(path).unwrap();
    AppTemplate::new(storage, runtime)
}
