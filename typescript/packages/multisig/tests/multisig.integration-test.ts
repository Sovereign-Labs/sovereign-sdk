import { Ed25519Signer, type Signer } from "@sovereign-sdk/signers";
import type { UnsignedTransaction } from "@sovereign-sdk/types";
import { bytesToHex } from "@sovereign-sdk/utils";
import {
  DEFAULT_TX_DETAILS,
  type StandardRollup,
  createStandardRollup,
} from "@sovereign-sdk/web3";
import { beforeAll, describe, expect, it } from "vitest";
import { MultisigTransaction } from "../src";

const testAddress = {
  Standard: "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
};

function generatePrivateKey(): Uint8Array {
  const array = new Uint8Array(32);
  crypto.getRandomValues(array);
  return array;
}

function generateSigners(count = 5): Signer[] {
  const signers = [];

  for (let i = 0; i < count; i++) {
    const pk = generatePrivateKey();
    const signer = new Ed25519Signer(pk);
    signers.push(signer);
  }

  return signers;
}

describe("multisig", async () => {
  let rollup: StandardRollup<any>;
  // gets populated in beforeAll
  let chain_id = 0;

  beforeAll(async () => {
    rollup = await createStandardRollup();
    const constants = await rollup.rollup.constants();
    chain_id = constants.chain_id;
  });

  it("should submit a multisig transaction successfully", async () => {
    const runtime_call = {
      bank: {
        create_token: {
          token_name: `multisig_test_${Date.now()}`,
          initial_balance: "50000",
          token_decimals: 12,
          supply_cap: "100000000000",
          mint_to_address: testAddress,
          admins: [testAddress],
        },
      },
    };
    const unsignedTx: UnsignedTransaction<any> = {
      runtime_call,
      uniqueness: { nonce: 0 },
      details: { ...DEFAULT_TX_DETAILS, chain_id },
    };

    const requiredSigners = 3;
    const multiSigSigners = generateSigners(requiredSigners);
    const allPublicKeyBytes = await Promise.all(
      multiSigSigners.map((s) => s.publicKey()),
    );
    const allPubKeys = allPublicKeyBytes.map(bytesToHex);
    const signedTransactions = await Promise.all(
      multiSigSigners.map((s) => rollup.signTransaction(unsignedTx, s)),
    );
    const multisig = MultisigTransaction.fromTransactions({
      txns: signedTransactions,
      minSigners: requiredSigners,
      allPubKeys,
    });
    const response = await rollup.submitTransaction(multisig.asTransaction());

    expect(response.status).toEqual("submitted");
  });
});
