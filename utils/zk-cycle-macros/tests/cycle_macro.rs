use std::marker::PhantomData;

use sov_zk_cycle_macros::cycle_tracker;

struct TestStruct;

impl TestStruct {
    #[cycle_tracker]
    fn _struct_method() {}

    #[cycle_tracker]
    fn _struct_method_with_self(&self) {}
}

struct TestStructGeneric<T, U> {
    _phantom_t: PhantomData<T>,
    _phantom_u: PhantomData<U>,
}

impl<T, U> TestStructGeneric<T, U> {
    #[cycle_tracker]
    fn _generic_method(&self, _t: T, _u: U) {}
}

#[cycle_tracker]
fn _generic_function<T, U>(_t: T, _u: U) {}

#[cycle_tracker]
fn _lifetime_function<'a>(s: &'a str) -> &'a str {
    s
}

struct TestStructLifetime<'a> {
    s: &'a str,
}

impl<'a> TestStructLifetime<'a> {
    #[cycle_tracker]
    fn _lifetime_method(&self) -> &'a str {
        self.s
    }
}

struct TestStructAssociated;

impl TestStructAssociated {
    #[cycle_tracker]
    fn _associated_function_no_self<T>(_value: T) {}
}

#[cycle_tracker]
fn _type_param_clause_function<T: Clone + 'static>(_t: T) {}

#[cycle_tracker]
fn _where_clause_function<T>(value: T)
where
    T: Clone + std::fmt::Debug,
{
    println!("{:?}", value.clone());
}

#[cycle_tracker]
fn _function_without_params() {}

#[cycle_tracker]
fn _function_with_params(_a: u32, _b: usize) {}

#[cycle_tracker]
pub fn _function_with_access_specifier(_a: u32, _b: usize) {}

fn main() {}
