//! Defines queries exposed by the accounts module, along with the relevant types

/// This is the response returned from the accounts_getAccount endpoint.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
#[serde(
    bound = "Addr: serde::Serialize + serde::de::DeserializeOwned",
    rename_all = "snake_case"
)]
pub enum Response<Addr> {
    /// The account corresponding to the given credential id exists.
    AccountExists {
        /// The address of the account,
        addr: Addr,
    },
    /// The account corresponding to the credential id does not exist.
    AccountEmpty,
}
