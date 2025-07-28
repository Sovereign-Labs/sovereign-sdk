use std::fs;

use crate::capabilities::config_chain_id;
use crate::{clap, Amount, CliWallet};

/// A trait that defines the interface for a CLI wallet.
pub trait CliFrontEnd<RT>
where
    RT: CliWallet,
{
    /// The intermediate representation of the CLI wallet.
    type CliIntermediateRepr<U>;
}

/// A trait that defines the arguments for a CLI transaction import method.
pub trait CliTxImportArg {
    /// The chain ID of the transaction.
    fn chain_id(&self) -> u64;

    /// The priority fee to pay the sequencer, expressed as a fraction of the tokens spent on gas in basis points.
    /// for example, setting this value to 1 pays a tip of 1 token to the sequencer for every 10_000 tokens spent on gas.
    /// similarly, setting this value to 50_000 pays 5 tokens to the sequencer for every token spent on gas
    fn max_priority_fee_bips(&self) -> u64;

    /// The max fee to pay for the transaction execution. This is the maximum amount expressed in gas tokens that can be
    /// charged for the gas fees of the transaction. This value contains both the tip and the base fee.
    /// It is mandatory to set this value as there is no associated default value.
    fn max_fee(&self) -> Amount;

    /// The gas limit for the transaction execution. This is an optional field that can be used to enforce a
    /// gas limit on the transaction execution - in a way that reproduces the behavior of the EIP-1559. If the gas limit is
    /// not provided, the transaction will be executed without checking the gas limit. The gas limit is a multi-dimensional gas vector
    /// that specify the maximum amount of gas that can be used along each dimension.
    /// If specified, up to `gas_limit *_scalar gas_price` tokens can be spent on gas execution in the transaction execution
    fn gas_limit(&self) -> Option<&[u64]>;
}

/// An argument to the cli containing a json string
#[derive(clap::Args, PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
pub struct JsonStringArg {
    /// The json formatted transaction data
    #[arg(long, help = "The JSON formatted transaction")]
    pub json: String,

    /// The chain ID of the transaction.
    #[arg(long, help = "The chain ID of the transaction.", default_value_t = config_chain_id())]
    pub chain_id: u64,

    /// the gas tip for the sequencer.
    #[arg(
        long,
        help = "The priority fee to pay the sequencer, expressed as a fraction of the tokens spent on gas in basis points.
        for example, setting this value to 1 pays a tip of 1 token to the sequencer for every 10_000 tokens spent on gas.
        similarly, setting this value to 50_000 pays 5 tokens to the sequencer for every token spent on gas",
        default_value = "0"
    )]
    pub max_priority_fee_bips: u64,

    /// The max fee to pay for the transaction execution.
    #[arg(
        long,
        help = "The max fee to pay for the transaction execution. This is the maximum amount expressed in gas tokens that can be
        charged for the gas fees of the transaction. This value contains both the tip and the base fee."
    )]
    pub max_fee: Amount,

    /// The gas limit for the transaction execution.
    #[arg(
        long,
        help = "The gas limit for the transaction execution. This is an optional field that can be used to enforce a
        gas limit on the transaction execution - in a way that reproduces the behavior of the EIP-1559. If the gas limit is
        not provided, the transaction will be executed without checking the gas limit. The gas limit is a multi-dimensional gas vector
        that specify the maximum amount of gas that can be used along each dimension.
        If specified, up to `gas_limit *_scalar gas_price` tokens can be spent on gas execution in the transaction execution",
        num_args = 0..
    )]
    pub gas_limit: Option<Vec<u64>>,
}

/// An argument to the cli containing a path to a file
#[derive(clap::Args, PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
pub struct FileNameArg {
    /// The json formatted transaction data
    #[arg(long, help = "The JSON formatted transaction")]
    pub path: String,

    /// The chain ID of the transaction.
    #[arg(long, help = "The chain ID of the transaction.", default_value_t = config_chain_id())]
    pub chain_id: u64,

    /// the gas tip for the sequencer.
    #[arg(
        long,
        help = "The priority fee to pay the sequencer, expressed as a fraction of the tokens spent on gas in basis points.
        for example, setting this value to 1 pays a tip of 1 token to the sequencer for every 10_000 tokens spent on gas.
        similarly, setting this value to 50_000 pays 5 tokens to the sequencer for every token spent on gas",
        default_value = "0"
    )]
    pub max_priority_fee_bips: u64,

    /// The max fee to pay for the transaction execution.
    #[arg(
        long,
        help = "The max fee to pay for the transaction execution. This is the maximum amount expressed in gas tokens that can be
        charged for the gas fees of the transaction. This value contains both the tip and the base fee."
    )]
    pub max_fee: Amount,

    /// The gas limit for the transaction execution.
    #[arg(
        long,
        help = "The gas limit for the transaction execution. This is an optional field that can be used to enforce a
        gas limit on the transaction execution - in a way that reproduces the behavior of the EIP-1559. If the gas limit is
        not provided, the transaction will be executed without checking the gas limit. The gas limit is a multi-dimensional gas vector
        that specify the maximum amount of gas that can be used along each dimension.
        If specified, up to `gas_limit *_scalar gas_price` tokens can be spent on gas execution in the transaction execution",
        num_args = 0..
    )]
    pub gas_limit: Option<Vec<u64>>,
}

impl CliTxImportArg for JsonStringArg {
    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    fn max_priority_fee_bips(&self) -> u64 {
        self.max_priority_fee_bips
    }

    fn max_fee(&self) -> Amount {
        self.max_fee
    }

    fn gas_limit(&self) -> Option<&[u64]> {
        self.gas_limit.as_deref()
    }
}

impl CliTxImportArg for FileNameArg {
    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    fn max_priority_fee_bips(&self) -> u64 {
        self.max_priority_fee_bips
    }

    fn max_fee(&self) -> Amount {
        self.max_fee
    }

    fn gas_limit(&self) -> Option<&[u64]> {
        self.gas_limit.as_deref()
    }
}

impl TryFrom<FileNameArg> for JsonStringArg {
    type Error = std::io::Error;
    fn try_from(arg: FileNameArg) -> Result<Self, Self::Error> {
        let FileNameArg {
            path,
            chain_id,
            max_priority_fee_bips,
            max_fee,
            gas_limit,
        } = arg;

        Ok(JsonStringArg {
            json: fs::read_to_string(path)?,
            chain_id,
            max_priority_fee_bips,
            max_fee,
            gas_limit,
        })
    }
}
