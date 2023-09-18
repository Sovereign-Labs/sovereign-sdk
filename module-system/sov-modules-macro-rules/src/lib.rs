#[macro_export]
macro_rules! define_offchain_function {
    (pub fn $func_name:ident($($arg:ident: $arg_type:ty),*) -> $return_type:ty { $($body:tt)* }) => {
        #[cfg(feature = "offchain")]
        pub fn $func_name($($arg: $arg_type),*) -> $return_type {
            $($body)*
        }

        #[cfg(not(feature = "offchain"))]
        pub fn $func_name(_dummy: ()) -> $return_type {
            // Do nothing
        }
    };
}

#[macro_export]
macro_rules! offchain_function {
    ($func_name:ident, $($arg:expr),*) => {
        #[cfg(feature = "offchain")]
        {
            $func_name($($arg),*)
        }

        #[cfg(not(feature = "offchain"))]
        {
            $func_name(())
        }
    };
}
