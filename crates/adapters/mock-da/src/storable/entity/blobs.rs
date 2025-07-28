//! Model for storing blobs
use std::convert::TryInto;

use sea_orm::entity::prelude::*;
use sea_orm::Set;

use crate::storable::entity::{BATCH_NAMESPACE, PROOF_NAMESPACE};
use crate::utils::hash_to_array;
use crate::{MockAddress, MockBlob, MockHash};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "blobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true, column_type = "Integer")]
    /// Unique ID of the blob. Used for ordering blobs inside single block.
    pub id: i32,
    /// Use i32 for compatibility with SQLite.
    pub block_height: i32,
    /// Stored as Vec<u8> because support for arrays is complicated.
    /// But always 32 bytes long.
    pub hash: Vec<u8>,
    /// Actual data of the blob.
    pub data: Vec<u8>,
    /// Which namespaces it belongs to.
    pub namespace: String,
    /// Who submitted it. Converted to `Vec<u8>` [`MockAddress`]
    pub sender: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

fn build_blob(
    height: i32,
    data: &[u8],
    sender: &MockAddress,
    namespace: String,
) -> (ActiveModel, MockHash) {
    let blob_hash = hash_to_array(data);
    (
        ActiveModel {
            block_height: Set(height),
            data: Set(data.to_vec()),
            sender: Set(sender.as_ref().to_vec()),
            namespace: Set(namespace),
            hash: Set(blob_hash.to_vec()),
            ..Default::default()
        },
        MockHash(blob_hash),
    )
}

pub fn build_batch_blob(height: i32, data: &[u8], sender: &MockAddress) -> (ActiveModel, MockHash) {
    build_blob(height, data, sender, BATCH_NAMESPACE.to_string())
}

pub fn build_proof_blob(height: i32, data: &[u8], sender: &MockAddress) -> (ActiveModel, MockHash) {
    build_blob(height, data, sender, PROOF_NAMESPACE.to_string())
}

impl From<Model> for MockBlob {
    fn from(value: Model) -> Self {
        let address =
            MockAddress::try_from(&value.sender[..]).expect("Malformed sender stored in database");
        let hash: [u8; 32] = value
            .hash
            .try_into()
            .expect("Blob hash should be 32 bytes long");
        MockBlob::new(value.data, address, hash)
    }
}
