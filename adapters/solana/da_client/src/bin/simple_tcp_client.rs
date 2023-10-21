use tokio::net::TcpStream;
use tokio::io::AsyncReadExt;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use account_proof_geyser::types::Update;


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect("127.0.0.1:10000").await?;

    let mut buffer = vec![0u8; 4096]; // Adjust the size based on your needs.

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
