use sov_modules_api::DispatchCall;
use sov_test_utils::runtime::{TestOptimisticRuntime, TestOptimisticRuntimeCallDiscriminants};
use sov_test_utils::TestSpec;
use strum::VariantArray;

#[test]
fn print_state_layout() {
    let runtime = TestOptimisticRuntime::<TestSpec>::default();
    let mut output = String::new();

    for (idx, discriminant) in TestOptimisticRuntimeCallDiscriminants::VARIANTS
        .iter()
        .enumerate()
    {
        if idx > 0 {
            output.push('\n');
        }

        let module_name = discriminant.as_ref();
        let module = runtime.module_info(*discriminant);
        output.push_str(&format!(
            "module {module_name} (discriminant {})\n",
            module.discriminant()
        ));

        for state_item in module.state_items() {
            output.push_str(&format!(
                "  state {} = {}\n",
                state_item.name, state_item.discriminant
            ));
        }
    }

    println!("{output}");
}
