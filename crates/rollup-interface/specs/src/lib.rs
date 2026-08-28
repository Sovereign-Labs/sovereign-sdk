#![doc = include_str!("../../README.md")]

#[cfg(doc)]
#[doc = include_str!("../../specs/overview.md")]
pub mod specs {
    #[doc = include_str!("../../specs/interfaces/interface.md")]
    pub mod interfaces {
        #[doc = include_str!("../../specs/interfaces/da.md")]
        pub mod da {}

        #[doc = include_str!("../../specs/interfaces/stf.md")]
        pub mod stf {}

        #[doc = include_str!("../../specs/interfaces/zkvm.md")]
        pub mod zkvm {}
    }
}
