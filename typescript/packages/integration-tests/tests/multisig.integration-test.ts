import { Multisig } from "@sovereign-sdk/multisig";
import { Ed25519Signer, type Signer } from "@sovereign-sdk/signers";
import { bytesToHex } from "@sovereign-sdk/utils";
import {
  DEFAULT_TX_DETAILS,
  type StandardRollup,
  createStandardRollup,
} from "@sovereign-sdk/web3";
import { beforeAll, describe, expect, it } from "vitest";

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
    signers.push(new Ed25519Signer(generatePrivateKey()));
  }

  return signers;
}

describe("multisig", async () => {
  let rollup: StandardRollup<any>;
  let chainId = 0;

  beforeAll(async () => {
    rollup = await createStandardRollup();
    const constants = await rollup.rollup.constants();
    chainId = constants.chain_id;
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
    const unsignedTx = {
      runtime_call,
      uniqueness: { nonce: 0 },
      details: { ...DEFAULT_TX_DETAILS, chain_id: chainId },
      address_override: null,
    };

    const requiredSigners = 3;
    const multiSigSigners = generateSigners(requiredSigners);
    const allPublicKeyBytes = await Promise.all(
      multiSigSigners.map((signer) => signer.publicKey()),
    );
    const multisig = Multisig.fromPubKeys(
      allPublicKeyBytes.map(bytesToHex),
      requiredSigners,
    );

    for (const signer of multiSigSigners) {
      const signingBytes = await rollup.multisigSigningBytes(unsignedTx, multisig);
      multisig.addSignature(
        bytesToHex(await signer.sign(signingBytes)),
        bytesToHex(await signer.publicKey()),
      );
    }

    const response = await rollup.submitTransaction(
      multisig.toTransaction(unsignedTx),
    );

    expect(response.status).toEqual("submitted");
  });
});
