pub trait Module {
    type GasConfig;
    fn foo(&self, gm: Self::GasConfig);
}

mod x {
    use std::marker::PhantomData;

    use super::Module;

    pub trait GasConfigX {
        fn x_gas(&self);
    }

    pub struct X<T> {
        pub _p: PhantomData<T>,
    }

    impl<T: GasConfigX> X<T> {
        pub fn lool(&self, gm: <Self as Module>::GasConfig) {
            gm.x_gas();
        }
    }

    impl<T: GasConfigX> Module for X<T> {
        type GasConfig = T;

        fn foo(&self, gm: Self::GasConfig) {
            gm.x_gas();
            self.lool(gm);
            todo!()
        }
    }
}

mod y {
    use std::marker::PhantomData;

    use super::{
        x::{GasConfigX, X},
        Module,
    };

    pub trait GasConfigY {
        fn y_gas(&self);
    }

    pub struct Y<T: TT> {
        x: X<T::G>,
    }

    pub trait TT {
        type G: GasConfigY + GasConfigX;
    }

    impl<T: TT> Module for Y<T> {
        type GasConfig = T::G;

        fn foo(&self, gm: Self::GasConfig) {
            gm.y_gas();
            self.x.lool(gm)
        }
    }
}
/*
mod runtime {
    use super::{
        x::{GasConfigX, X},
        y::{GasConfigY, Y},
        Module,
    };

    struct GasC {
        x: GasConfigX,
        y: GasConfigY,
    }

    impl AsRef<GasConfigX> for GasC {
        fn as_ref(&self) -> &GasConfigX {
            &self.x
        }
    }

    impl AsRef<GasConfigY> for GasC {
        fn as_ref(&self) -> &GasConfigY {
            &self.y
        }
    }

    fn fff() {
        let gm = GasC {
            x: todo!(),
            y: todo!(),
        };

        let x = X::<GasC> {
            _p: std::marker::PhantomData,
        };

        x.foo(gm)
    }
}*/
/*trait Module {
    type GasConfig;
    fn foo(&self, gm: &dyn AsRef<Self::GasConfig>);
}

//===

mod x {
    use super::Module;

    pub struct GasConfigX {
        x_gas: u64,
    }
    pub struct X {}

    impl X {
        pub fn lol(&self, gm: &dyn AsRef<<Self as Module>::GasConfig>) {}
    }

    impl Module for X {
        type GasConfig = GasConfigX;

        fn foo(&self, gm: &dyn AsRef<Self::GasConfig>) {
            let g = gm.as_ref().x_gas;
        }
    }
}

mod y {
    use super::{x::X, Module};

    ///==
    pub struct GasConfigY {
        y_gas: u64,
    }
    pub struct Y {
        x: X,
    }

    impl Module for Y {
        type GasConfig = GasConfigY;

        fn foo(&self, gm: &dyn AsRef<Self::GasConfig>) {
            let g = gm.as_ref().y_gas;
            self.x.lol(gm)
        }
    }
}

mod runtime {
    use super::{x::X, y::Y, Module};

    struct GasC {
        x: <X as Module>::GasConfig,
        y: <Y as Module>::GasConfig,
    }

    impl AsRef<<X as Module>::GasConfig> for GasC {
        fn as_ref(&self) -> &<X as Module>::GasConfig {
            &self.x
        }
    }

    impl AsRef<<Y as Module>::GasConfig> for GasC {
        fn as_ref(&self) -> &<Y as Module>::GasConfig {
            &self.y
        }
    }

    fn fff() {
        let x = X {};
        let gm = GasC {
            x: todo!(),
            y: todo!(),
        };

        x.foo(&gm)
    }
}*/

/*
trait Module {
    type GasConfig;
    fn foo<G: AsRef<Self::GasConfig>>(&self, gm: &G);
}

mod x {
    use super::Module;

    pub struct GasConfigX {
        x_gas: u64,
    }
    pub struct X {}

    impl Module for X {
        type GasConfig = GasConfigX;

        fn foo<G: AsRef<Self::GasConfig>>(&self, gm: &G) {
            let g = gm.as_ref().x_gas;
        }
    }
}

mod y {
    use super::{x::X, Module};

    ///==
    pub struct GasConfigY {
        y_gas: u64,
    }
    pub struct Y {
        x: X,
    }

    impl Module for Y {
        type GasConfig = GasConfigY;

        fn foo<G: AsRef<Self::GasConfig> + AsRef<<X as Module>::GasConfig>>(&self, gm: &G) {
            let g = gm.as_ref().y_gas;
            self.x.foo(gm);
        }
    }
}

mod runtime {
    use super::{x::X, Module};

    struct GasC {
        x: <X as Module>::GasConfig,
        //    y: <Y as Module>::GasConfig,
    }

    impl AsRef<<X as Module>::GasConfig> for GasC {
        fn as_ref(&self) -> &<X as Module>::GasConfig {
            &self.x
        }
    }

    /*
    impl AsRef<<Y as Module>::GasConfig> for GasC {
        fn as_ref(&self) -> &<Y as Module>::GasConfig {
            &self.y
        }
    }*/

    fn fff() {
        let x = X {};
        let gm = GasC {
            x: todo!(),
            //   y: todo!(),
        };

        x.foo(&gm)
    }
}
*/
