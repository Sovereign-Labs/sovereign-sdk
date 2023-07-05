use std::{convert::Infallible, hash::Hash};

use ethers::{
    types::{transaction::eip2718::TypedTransaction, Block, Bytes, FeeHistory, TxHash, U64},
    utils::{
        hex,
        rlp::{self, Rlp},
    },
};
use jsonrpsee::RpcModule;

pub struct Ethereum {}

pub fn get_ethereum_rpc() -> RpcModule<Ethereum> {
    let e = Ethereum {};
    let mut rpc = RpcModule::new(e);
    register_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");

    rpc
}

fn register_rpc_methods(rpc: &mut RpcModule<Ethereum>) -> Result<(), jsonrpsee::core::Error> {
    rpc.register_method::<Result<(), ()>, _>("eth_sendTransaction", |p, e| {
        println!("eth_sendTransaction");
        unimplemented!()
    })?;

    rpc.register_method("eth_blockNumber", |p, e| {
        println!("eth_blockNumber");
        Ok(unimplemented!())
    })?;

    rpc.register_method("eth_getTransactionByHash", |p, e| {
        println!("eth_getTransactionByHash");
        Ok(unimplemented!())
    })?;

    rpc.register_method("eth_getTransactionReceipt", |p, e| {
        println!("eth_getTransactionReceipt");
        Ok(unimplemented!())
    })?;

    rpc.register_method("get_block_number", |p, e| {
        println!("get_block_number");
        Ok(unimplemented!())
    })?;

    rpc.register_method("eth_sendRawTransaction", |p, e| {
        println!("eth_sendRawTransaction");
        let data: Bytes = p.one().unwrap();
        let data = data.as_ref();

        if data[0] > 0x7f {
            panic!("lol")
        }

        let r = Rlp::new(data);

        let (decoded_tx, _decoded_sig) = TypedTransaction::decode_signed(&r).unwrap();
        println!("decoded_tx {:?}", decoded_tx);

        let h: TxHash = decoded_tx.sighash();
        Ok(h)
    })?;

    rpc.register_method("eth_getTransactionCount", |p, e| Ok(unimplemented!()))?;

    rpc.register_method("eth_chainId", |params, e| {
        println!("eth_chainId");
        Ok(Some(U64::from(1u64)))
    })?;

    rpc.register_method("eth_getBlockByNumber", |params, e| {
        println!("eth_getBlockByNumber");

        let mut seq = params.sequence();
        let b: &str = seq.next().unwrap();
        let l: bool = seq.next().unwrap();

        println!("{} {}", b, l);

        let b = Block::<TxHash> {
            base_fee_per_gas: Some(100.into()),
            ..Default::default()
        };

        Ok(Some(b))
    })?;

    rpc.register_method("eth_feeHistory", |p, e| {
        println!("eth_feeHistory");

        let fh = FeeHistory {
            base_fee_per_gas: Default::default(),
            gas_used_ratio: Default::default(),
            oldest_block: Default::default(),
            reward: Default::default(),
        };

        Ok(fh)
    })?;

    Ok(())
}
