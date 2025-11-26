import { ethers } from "ethers";

async function main() {
  const rpcUrl = process.env.RPC_URL;
  const privateKey = process.env.PRIVATE_KEY;
  const numTxs = parseInt(process.env.NUM_TXS || "3");

  if (!rpcUrl || !privateKey) {
    console.error("Usage: RPC_URL=<url> PRIVATE_KEY=<key> [NUM_TXS=3] node test-nonce.js");
    process.exit(1);
  }

  const provider = new ethers.JsonRpcProvider(rpcUrl);
  const wallet = new ethers.Wallet(privateKey, provider);
  const address = wallet.address;

  console.log(`Address: ${address}`);
  console.log(`Sending ${numTxs} self-transfers\n`);

  async function printNonces(label) {
    const latest = await provider.getTransactionCount(address, "latest");
    const pending = await provider.getTransactionCount(address, "pending");
    const withoutReference = await provider.getTransactionCount(address);
    const rawLatest = await provider.send("eth_getTransactionCount", [address, "latest"]);
    const rawPending = await provider.send("eth_getTransactionCount", [address, "pending"]);
    console.log(`[${label}] latest: ${latest}, pending: ${pending}, withoutReference: ${withoutReference} | raw: ${parseInt(rawLatest, 16)}, ${parseInt(rawPending, 16)}`);
  }

  await printNonces("initial");

  for (let i = 0; i < numTxs; i++) {
    const tx = await wallet.sendTransaction({
      to: address,
      value: 0,
    });
    console.log(`\nTx ${i + 1} sent: ${tx.hash}`);
    
    await printNonces("after send");
    
    const receipt = await tx.wait();
    console.log(`Tx ${i + 1} confirmed in block ${receipt.blockNumber}`);
    
    await printNonces("after confirm");
    console.log("\n\n\n");
  }

  console.log("\nDone.");
}

main().catch(console.error);