#[cfg(feature = "native")]
mod native;

#[cfg(feature = "native")]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    native::main().await
}

#[cfg(not(feature = "native"))]
fn main() -> Result<(), anyhow::Error> {
    Err(anyhow::format_err!("CLI support is only available when the app is compiled with the 'native' flag. You can recompile with 'cargo build --features=native' to use the CLI"))
}
