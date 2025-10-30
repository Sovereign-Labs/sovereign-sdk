use crate::token::TokenId;
use sov_modules_api::{err_detail, Amount, CoreModuleError, ErrorContext, ErrorDetail};

/// Errors that occur during arithmetic operations on token amounts.
///
/// These errors are returned when mathematical operations (addition, subtraction)
/// on token balances or supplies would result in overflow or underflow conditions.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum ArithmeticError {
    /// An arithmetic overflow occurred during addition.
    #[error("Overflow occurred: {message}, {base} + {addend}")]
    Overflow {
        /// The base amount before the addition operation.
        base: Amount,
        /// The amount being added to the base.
        addend: Amount,
        /// A descriptive message about the overflow context.
        message: String,
    },
    /// An arithmetic underflow occurred during subtraction.
    #[error("Underflow occurred: {message}, {base} - {subtrahend}")]
    Underflow {
        /// The base amount before the subtraction operation.
        base: Amount,
        /// The amount being subtracted from the base.
        subtrahend: Amount,
        /// A descriptive message about the underflow context.
        message: String,
    },
}

/// Common errors that can occur across different bank operations.
///
/// These errors represent shared failure conditions that can happen during
/// various token operations like transfers, mints, burns, and administrative actions.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum CommonError {
    /// An error occurred in a core module operation.
    #[error(transparent)]
    CoreModuleError(#[from] CoreModuleError),
    /// An arithmetic error occurred during a mathematical operation.
    #[error(transparent)]
    Arithmetic(#[from] ArithmeticError),
    /// The caller is not authorized to perform the requested operation on this token.
    #[error("Caller {caller} is not an admin for token {token_name}")]
    NotAdmin {
        /// The address of the caller who attempted the operation.
        caller: String,
        /// The name of the token they tried to access.
        token_name: String,
    },
    /// The requested token does not exist.
    #[error("Token not found: {token_id}")]
    TokenNotFound {
        /// The ID of the token that was not found.
        token_id: TokenId,
    },
    /// The token is frozen and cannot be operated on.
    #[error("Token is frozen: {name}")]
    TokenFrozen {
        /// The name of the frozen token.
        name: String,
    },
}

/// Errors that can occur during token creation.
///
/// These errors are returned when attempting to create a new token fails due to
/// validation issues, conflicting tokens, or other constraints.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum CreateTokenError {
    /// A common error occurred during token creation.
    #[error(transparent)]
    Common(#[from] CommonError),
    /// The specified number of decimal places exceeds the maximum allowed.
    #[error("Too many decimals: {provided}, max allowed is {max_allowed}")]
    TooManyDecimals {
        /// The number of decimals provided in the request.
        provided: u8,
        /// The maximum number of decimals allowed.
        max_allowed: u8,
    },
    /// The initial balance exceeds the token's supply cap.
    #[error(
        "Requested initial balance {initial_balance} is greater than the supply cap {supply_cap}"
    )]
    InitialBalanceExceedsSupplyCap {
        /// The requested initial balance for the token.
        initial_balance: Amount,
        /// The maximum supply cap for the token.
        supply_cap: Amount,
    },
    /// A token with the same ID already exists.
    #[error("Token with id already exists {token_id}, name={name} minter={minter}")]
    TokenAlreadyExists {
        /// The ID of the existing token.
        token_id: String,
        /// The name of the existing token.
        name: String,
        /// The address of the token minter.
        minter: String,
    },
}

/// Errors that can occur during token transfers.
///
/// These errors are returned when attempting to transfer tokens fails due to
/// insufficient balances, invalid tokens, or other transfer constraints.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum TransferTokenError {
    /// A common error occurred during token transfer.
    #[error(transparent)]
    Common(#[from] CommonError),
    /// The sender has insufficient balance to complete the transfer.
    #[error("Insufficient balance: amount={amount}, balance={balance}, from={from}, to={to}, token_id={token_id}")]
    InsufficientBalance {
        /// The amount requested to transfer.
        amount: Amount,
        /// The actual balance available.
        balance: Amount,
        /// The sender's address.
        from: String,
        /// The recipient's address.
        to: String,
        /// The ID of the token being transferred.
        token_id: String,
    },
}

/// Errors that can occur during token freezing operations.
///
/// These errors are returned when attempting to freeze a token fails due to
/// authorization issues or other constraints.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum FreezeTokenError {
    /// A common error occurred during token freezing.
    #[error(transparent)]
    Common(#[from] CommonError),
}

/// Errors that can occur during token burning operations.
///
/// These errors are returned when attempting to burn (permanently destroy) tokens
/// fails due to insufficient balance, invalid tokens, or other constraints.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum BurnTokenError {
    /// A common error occurred during token burning.
    #[error(transparent)]
    Common(#[from] CommonError),
}

/// Errors that can occur during token minting operations.
///
/// These errors are returned when attempting to mint new tokens fails due to
/// authorization issues, supply cap violations, or other constraints.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum MintTokenError {
    /// A common error occurred during token minting.
    #[error(transparent)]
    Common(#[from] CommonError),
    /// The minting operation would exceed the token's supply cap.
    #[error("Minting this amount would exceed the supply cap: supply_cap={supply_cap}, new_supply={new_supply}")]
    SupplyCapExceeded {
        /// The maximum allowed supply for the token.
        supply_cap: Amount,
        /// The total supply that would result from the mint operation.
        new_supply: Amount,
        /// The name of the token.
        token_name: String,
    },
}

/// Errors that can occur during admin update operations.
///
/// These errors are returned when attempting to add or remove token administrators
/// fails due to validation issues or existing state conflicts.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "error_code", rename_all = "snake_case")]
pub enum UpdateAdminError {
    /// A common error occurred during admin update.
    #[error(transparent)]
    Common(#[from] CommonError),
    /// The admin to be replaced does not exist for this token.
    #[error("Admin to replace {admin_to_replace} does not exist for token {token_name}")]
    AdminDoesNotExist {
        /// The address of the admin that was expected to exist.
        admin_to_replace: String,
        /// The name of the token.
        token_name: String,
    },
    /// The admin to be added already exists for this token.
    #[error("Admin {new_admin} already exists for token {token_name}")]
    AdminAlreadyExists {
        /// The address of the admin that already exists.
        new_admin: String,
        /// The name of the token.
        token_name: String,
    },
}

/// The top-level error type for all bank module operations.
///
/// This enum wraps all specific error types that can occur during different
/// bank operations, providing a unified error interface for the module.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Error {
    /// An error occurred during token creation.
    #[error("Token creation error: {0}")]
    CreateToken(#[from] CreateTokenError),
    /// An error occurred during token transfer.
    #[error("Token transfer error: {0}")]
    TransferToken(#[from] TransferTokenError),
    /// An error occurred during token freezing.
    #[error("Token freeze error: {0}")]
    FreezeToken(#[from] FreezeTokenError),
    /// An error occurred during token burning.
    #[error("Token burn error: {0}")]
    BurnToken(#[from] BurnTokenError),
    /// An error occurred during token minting.
    #[error("Token mint error: {0}")]
    MintToken(#[from] MintTokenError),
    /// An error occurred during admin update.
    #[error("Token update admin error: {0}")]
    UpdateAdmin(#[from] UpdateAdminError),
}

impl ErrorDetail for Error {
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(err_detail!(self))
    }
}

macro_rules! impl_from_core_errors {
    ($error_type:ty) => {
        impl From<::sov_modules_api::CoreModuleError> for $error_type {
            fn from(err: ::sov_modules_api::CoreModuleError) -> Self {
                CommonError::from(err).into()
            }
        }

        impl From<anyhow::Error> for $error_type {
            fn from(err: anyhow::Error) -> Self {
                CommonError::from(::sov_modules_api::CoreModuleError::Generic(err)).into()
            }
        }
    };
}

impl_from_core_errors!(CreateTokenError);
impl_from_core_errors!(TransferTokenError);
impl_from_core_errors!(FreezeTokenError);
impl_from_core_errors!(BurnTokenError);
impl_from_core_errors!(MintTokenError);
impl_from_core_errors!(UpdateAdminError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arithmetic_error_details() {
        let err = Error::CreateToken(CreateTokenError::Common(CommonError::Arithmetic(
            ArithmeticError::Overflow {
                base: Amount(100),
                addend: Amount(50),
                message: "Addition overflow".to_string(),
            },
        )));
        let detail = err.error_detail().unwrap();
        let expected = err_detail!({
            "call": "create_token",
            "error_code": "overflow",
            "base": "100",
            "addend": "50",
            "message": "Addition overflow"
        });
        assert_eq!(detail, expected);
    }

    #[test]
    fn test_create_token_too_many_decimals() {
        let err = Error::CreateToken(CreateTokenError::TooManyDecimals {
            provided: 20,
            max_allowed: 18,
        });
        let detail = err.error_detail().unwrap();
        let expected = err_detail!({
            "call": "create_token",
            "error_code": "too_many_decimals",
            "provided": 20,
            "max_allowed": 18
        });
        assert_eq!(detail, expected);
    }

    #[test]
    fn test_transfer_insufficient_balance() {
        let err = Error::TransferToken(TransferTokenError::InsufficientBalance {
            amount: Amount(100),
            balance: Amount(50),
            from: "addr1".to_string(),
            to: "addr2".to_string(),
            token_id: "token123".to_string(),
        });
        let detail = err.error_detail().unwrap();
        let expected = err_detail!({
            "call": "transfer_token",
            "error_code": "insufficient_balance",
            "amount": "100",
            "balance": "50",
            "from": "addr1",
            "to": "addr2",
            "token_id": "token123"
        });
        assert_eq!(detail, expected);
    }

    #[test]
    fn test_mint_supply_cap_exceeded() {
        let err = Error::MintToken(MintTokenError::SupplyCapExceeded {
            supply_cap: Amount(1000),
            new_supply: Amount(1500),
            token_name: "TestToken".to_string(),
        });
        let detail = err.error_detail().unwrap();
        let expected = err_detail!({
            "call": "mint_token",
            "error_code": "supply_cap_exceeded",
            "supply_cap": "1000",
            "new_supply": "1500",
            "token_name": "TestToken"
        });
        assert_eq!(detail, expected);
    }

    #[test]
    fn test_update_admin_does_not_exist() {
        let err = Error::UpdateAdmin(UpdateAdminError::AdminDoesNotExist {
            admin_to_replace: "old_admin".to_string(),
            token_name: "TestToken".to_string(),
        });
        let detail = err.error_detail().unwrap();
        let expected = err_detail!({
            "call": "update_admin",
            "error_code": "admin_does_not_exist",
            "admin_to_replace": "old_admin",
            "token_name": "TestToken"
        });
        assert_eq!(detail, expected);
    }

    #[test]
    fn test_common_error_not_admin() {
        let err = Error::CreateToken(CreateTokenError::Common(CommonError::NotAdmin {
            caller: "unauthorized_user".to_string(),
            token_name: "TestToken".to_string(),
        }));
        let detail = err.error_detail().unwrap();
        let expected = err_detail!({
            "call": "create_token",
            "error_code": "not_admin",
            "caller": "unauthorized_user",
            "token_name": "TestToken"
        });
        assert_eq!(detail, expected);
    }
}
