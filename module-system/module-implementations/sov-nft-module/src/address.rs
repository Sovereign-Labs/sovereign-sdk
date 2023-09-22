use std::fmt;

use sov_modules_api::Context;
use sov_modules_macros::address_type;

#[address_type]
/// Address representing a simple user capable of owning an NFT
pub struct UserAddress;
#[address_type]
/// Derived Address representing an NFT collection - Derived from CreatorAddress(C::Address) and collection_name: String
pub struct CollectionAddress;
#[address_type]
/// Address representing the owner of an NFT
pub struct OwnerAddress;
#[address_type]
/// Address representing a creator of an NFT collection
pub struct CreatorAddress;
