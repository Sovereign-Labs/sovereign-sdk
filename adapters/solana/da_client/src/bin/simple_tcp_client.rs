use account_proof_geyser::types::Update;
use account_proof_geyser::utils::verify_proof;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect("127.0.0.1:10000").await?;

    let mut buffer = vec![0u8; 4096]; // Buffer needs to be raised

    loop {
        let n = stream.read(&mut buffer).await?;

        if n == 0 {
            break; // Connection closed.
        }

        let received_update: Update = Update::try_from_slice(&buffer[..n])?;
        println!("{:?}", received_update);
    }

    Ok(())
}
