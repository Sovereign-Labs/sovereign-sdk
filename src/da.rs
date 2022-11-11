pub trait DaApp {
    type Blockhash;
    type Address;
    type Header;
    type Transaction: TxWithSender<Self::Address>;
    /// A proof that a particular transaction is included in a block
    type InclusionProof;
    /// A proof that a set of transactions are included in a block
    type InclusionMultiProof;
    /// A proof that a *claimed* set of transactions is complete relative to
    /// some selection function supported by the DA layer. For example, this could be a range
    /// proof for an entire Celestia namespace.
    type CompletenessProof;
}

pub trait TxWithSender<Addr> {
    fn sender(&self) -> &Addr;
}
