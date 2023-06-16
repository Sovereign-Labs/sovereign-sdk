use bytes::Bytes;
use ethereum_types::U256 as EU256;
use ethers_contract::BaseContract;
use ethers_core::abi::Abi;
use revm::{
    primitives::{
        Account, AccountInfo, Bytecode, HashMap, TransactTo, B160, B256, KECCAK_EMPTY, U256,
    },
    Database, DatabaseCommit, DummyStateDB,
};
use std::str::FromStr;

struct Foo<'s> {
    x: &'s u32,
}

impl<'a> Database for Foo<'a> {
    type Error = ();

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: B160) -> Result<Option<AccountInfo>, Self::Error> {
        todo!()
    }

    #[doc = " Get account code by its hash"]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        todo!()
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: B160, index: U256) -> Result<U256, Self::Error> {
        todo!()
    }

    fn block_hash(&mut self, number: U256) -> Result<B256, Self::Error> {
        todo!()
    }
}

impl<'a> DatabaseCommit for Foo<'a> {
    fn commit(&mut self, changes: HashMap<B160, Account>) {
        todo!()
    }
}

// solc --abi --bin  Store.sol -o build

fn simple_storage() {
    let caller = B160::from_str("0x1000000000000000000000000000000000000000").unwrap();
    let mut evm = revm::new();
    let mut db = DummyStateDB::default();

    db.insert_account_info(
        caller,
        AccountInfo {
            nonce: 1,
            balance: U256::from(1000000000),
            code: None,
            code_hash: KECCAK_EMPTY,
        },
    );

    let x = 22;
    let foo = Foo { x: &x };
    evm.database(foo);

    evm.env.tx.transact_to = TransactTo::create();

    let data = std::fs::read_to_string("../sol/build/SimpleStorage.bin").unwrap();
    evm.env.tx.data = Bytes::from(hex::decode(data).unwrap());

    let res = evm.transact_commit().unwrap();

    let contract_address = match res {
        revm::primitives::ExecutionResult::Success {
            reason,
            gas_used,
            gas_refunded,
            logs,
            output,
        } => match output {
            revm::primitives::Output::Call(_) => todo!(),
            revm::primitives::Output::Create(_, addr) => addr.unwrap(),
        },
        revm::primitives::ExecutionResult::Revert { gas_used, output } => todo!(),
        revm::primitives::ExecutionResult::Halt { reason, gas_used } => todo!(),
    };

    println!("{:?}", contract_address);

    let abi_json = std::fs::read_to_string("../sol/build/SimpleStorage.abi").unwrap();
    let abi: Abi = serde_json::from_str(&abi_json).unwrap();
    let abi = BaseContract::from(abi);
    {
        let x = EU256::from(21989);
        let encoded = abi.encode("set", x).unwrap();

        //
        evm.env.tx.transact_to = TransactTo::Call(contract_address);
        // todo
        evm.env.tx.data = Bytes::from(hex::decode(hex::encode(&encoded)).unwrap());
        let r = evm.transact_commit().unwrap();
        //println!("{:?}", r)
    }

    {
        let encoded = abi.encode("get", ()).unwrap();

        //
        evm.env.tx.transact_to = TransactTo::Call(contract_address);
        // todo
        evm.env.tx.data = Bytes::from(hex::decode(hex::encode(&encoded)).unwrap());
        let r = evm.transact_commit().unwrap();
        let o: &[u8] = r.output().unwrap().as_ref();

        let r = EU256::from(o);
        println!("{:?}", r);
    }
}

fn main() {
    simple_storage();
}
