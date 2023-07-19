use std::fs::File;
use std::io::{self, Write};

use sov_modules_api::default_context::DefaultContext as C;
use sov_modules_api::ModuleCallJsonSchema;

fn main() -> io::Result<()> {
    store_json_schema::<sov_bank::Bank<C>>("sov-bank.json")?;
    Ok(())
}

fn store_json_schema<M: ModuleCallJsonSchema>(filename: &str) -> io::Result<()> {
    let mut file = File::create(format!("schemas/{}", filename))?;
    file.write_all(M::json_schema().as_bytes())?;
    Ok(())
}
