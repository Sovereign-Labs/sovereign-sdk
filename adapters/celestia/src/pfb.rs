/// BlobTx wraps an encoded sdk.Tx with a second field to contain blobs of data.
/// The raw bytes of the blobs are not signed over, instead we verify each blob
/// using the relevant MsgPayForBlobs that is signed over in the encoded sdk.Tx.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BlobTx {
    #[prost(bytes = "bytes", tag = "1")]
    pub tx: ::prost::bytes::Bytes,
}

/// Tx is the standard type used for broadcasting transactions.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Tx {
    /// body is the processable content of the transaction
    #[prost(message, optional, tag = "1")]
    pub body: ::core::option::Option<TxBody>,
}

/// TxBody is the body of a transaction that all signers sign over.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TxBody {
    /// messages is a list of messages to be executed. The required signers of
    /// those messages define the number and order of elements in AuthInfo's
    /// signer_infos and Tx's signatures. Each required signer address is added to
    /// the list only the first time it occurs.
    /// By convention, the first required signer (usually from the first message)
    /// is referred to as the primary signer and pays the fee for the whole
    /// transaction.
    #[prost(message, repeated, tag = "1")]
    pub messages: ::prost::alloc::vec::Vec<::prost_types::Any>,
}

// @generated
/// MsgPayForBlobs pays for the inclusion of a blob in the block.
#[derive(
    Clone,
    PartialEq,
    ::prost::Message,
    serde::Deserialize,
    serde::Serialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
)]
pub struct MsgPayForBlobs {
    #[prost(string, tag = "1")]
    pub signer: ::prost::alloc::string::String,
    #[prost(bytes = "bytes", repeated, tag = "2")]
    pub namespace_ids: ::prost::alloc::vec::Vec<::prost::bytes::Bytes>,
    #[prost(uint32, repeated, tag = "3")]
    pub blob_sizes: ::prost::alloc::vec::Vec<u32>,
    /// share_commitments is a list of share commitments (one per blob).
    #[prost(bytes = "bytes", repeated, tag = "4")]
    pub share_commitments: ::prost::alloc::vec::Vec<::prost::bytes::Bytes>,
    /// share_versions are the versions of the share format that the blobs
    /// associated with this message should use when included in a block. The
    /// share_versions specified must match the share_versions used to generate the
    /// share_commitment in this message.
    #[prost(uint32, repeated, tag = "8")]
    pub share_versions: ::prost::alloc::vec::Vec<u32>,
}
// @@protoc_insertion_point(module)
