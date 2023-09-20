use std::fmt;
use sov_modules_api::Context;
use sov_modules_macros::address_type;

#[address_type]
pub struct UserAddress;
#[address_type]
pub struct CollectionAddress;
#[address_type]
pub struct OwnerAddress;
#[address_type]
pub struct CreatorAddress;