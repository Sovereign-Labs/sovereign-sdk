#![no_main]
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

pub fn main() {
    // TODO: Implement your guest code here

    // read the input
    let mut input: u32 = env::read();

    // TODO: do something with the input
    for i in 1..10000 {
        input *= i;
        input += i;
        input /= i;
    }

    // write public output to the journal
    env::commit(&input);
}
