#[cfg(feature = "native")]
mod native;
#[cfg(feature = "native")]

#[cfg(feature = "native")]
fn main() {
    native::main()
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("non native binary. cli only works with native");
}



