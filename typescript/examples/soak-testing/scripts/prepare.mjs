import * as ed25519 from "@noble/ed25519";
import { addressFromPublicKey } from "@sovereign-sdk/web3";
import { randomBytes } from "crypto";
import { copyFileSync, writeFileSync } from "fs";
import genesis from "../templates/genesis.json" assert { type: "json" };

async function generateKeypairs(count = 100) {
  const keypairs = [];

  for (let i = 0; i < count; i++) {
    const privateKey = randomBytes(32);
    const publicKey = await ed25519.getPublicKeyAsync(privateKey);
    const keypair = {
      id: i + 1,
      privateKey: Buffer.from(privateKey).toString("hex"),
      publicKey: Buffer.from(publicKey).toString("hex"),
      address: addressFromPublicKey(publicKey, "sov"),
    };

    keypairs.push(keypair);
  }

  return keypairs;
}

(async () => {
  try {
    const keypairs = await generateKeypairs(100);
    writeFileSync("data/keypairs.json", JSON.stringify(keypairs, null, 2));

    for (const { address } of keypairs) {
      genesis.bank.gas_token_config.address_and_balances.push([
        address,
        "10000000000000000",
      ]);
    }

    writeFileSync("data/genesis.json", JSON.stringify(genesis, null, 2));
    copyFileSync("templates/rollup.toml", "data/rollup.toml");

    console.log(
      "Genesis, rollup config & keypairs written to 'data' directory"
    );
  } catch (error) {
    console.error("❌ Error setting up data:", error);
  }
})();
