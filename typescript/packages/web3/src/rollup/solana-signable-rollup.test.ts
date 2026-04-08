import { sha256 } from "@noble/hashes/sha2";
import SovereignClient from "@sovereign-sdk/client";
import { JsSerializer } from "@sovereign-sdk/serializers";
import { Ed25519Signer } from "@sovereign-sdk/signers";
import { LedgerSolanaSigner } from "@sovereign-sdk/signers/ledger-solana";
import { bytesToHex, hexToBytes } from "@sovereign-sdk/utils";
import { describe, expect, it, vi } from "vitest";
import demoRollupSchema from "../../../__fixtures__/demo-rollup-schema.json";
import {
  SolanaSignableRollup,
  createSolanaSignableRollup,
} from "./solana-signable-rollup";

function createMockClient(overrides?: {
  chainId?: number;
  chainName?: string;
  chainHash?: string;
}) {
  const mockClient = new SovereignClient({ fetch: vi.fn() });
  const chainId = overrides?.chainId || 1;
  const chainName = overrides?.chainName ?? "";

  mockClient.rollup = {
    constants: vi.fn().mockResolvedValue({ chain_id: chainId }),
    schema: vi.fn().mockResolvedValue({
      schema: {
        chain_data: {
          chain_id: chainId,
          chain_name: chainName,
        },
      },
      chain_hash:
        overrides?.chainHash ||
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    }),
  } as any;

  return mockClient;
}

function createMockSerializer(overrides?: any) {
  return {
    serializeUnsignedTx: vi.fn().mockReturnValue(new Uint8Array(10)),
    serializeTx: vi.fn().mockReturnValue(new Uint8Array(10)),
    serializeRuntimeCall: vi.fn().mockReturnValue(new Uint8Array(10)),
    schema: overrides?.schema || {},
    ...overrides,
  } as any;
}

function createMockSigner() {
  return {
    sign: vi.fn().mockResolvedValue(new Uint8Array(64)),
    publicKey: vi.fn().mockResolvedValue(new Uint8Array(32)),
  } as any;
}

describe("SolanaSignableRollup", () => {
  it("should create a SolanaSignableRollup instance", async () => {
    const mockClient = createMockClient();
    const rollup = await createSolanaSignableRollup({ client: mockClient });
    expect(rollup).toBeInstanceOf(SolanaSignableRollup);
  });

  it("should delegate all StandardRollup methods", async () => {
    const mockClient = createMockClient();

    // Mock the post method for transaction submission
    mockClient.post = vi.fn().mockResolvedValue({ id: "test-hash" });

    const rollup = await createSolanaSignableRollup({
      client: mockClient,
      getSerializer: () => createMockSerializer(),
    });

    // Check that key methods are available
    expect(typeof rollup.call).toBe("function");
    expect(typeof rollup.signAndSubmitTransaction).toBe("function");
    expect(typeof rollup.simulate).toBe("function");
    expect(typeof rollup.serializer).toBe("function");
    expect(typeof rollup.submitTransaction).toBe("function");

    // Verify that standard authenticator works
    const result = await rollup.call({ test: "runtime" } as any, {
      signer: createMockSigner(),
      authenticator: "standard",
    });

    expect(result).toHaveProperty("response");
    expect(result).toHaveProperty("transaction");
    expect(result.response.id).toBe("test-hash");
  });

  it("should pass configuration to createStandardRollup", async () => {
    const mockClient = createMockClient({ chainId: 42 });

    const customConfig = {
      client: mockClient,
      context: {
        defaultTxDetails: {
          chain_id: 42,
          max_priority_fee_bips: 100,
          max_fee: "200000000",
          gas_limit: null,
        },
      },
    };

    const rollup = await createSolanaSignableRollup(customConfig);

    expect(rollup.context.defaultTxDetails.chain_id).toBe(42);
    expect(rollup.context.defaultTxDetails.max_priority_fee_bips).toBe(100);
    expect(rollup.context.defaultTxDetails.max_fee).toBe("200000000");
  });

  it("should work without any configuration", async () => {
    const mockClient = createMockClient();

    const rollup = await createSolanaSignableRollup({ client: mockClient });

    expect(rollup).toBeInstanceOf(SolanaSignableRollup);
    expect(rollup.context.defaultTxDetails.chain_id).toBe(1);
  });

  it("should allow custom Solana endpoint configuration", async () => {
    const mockClient = createMockClient();
    const customEndpoint = "/custom/solana-tx-endpoint";

    // Capture the endpoint and payload sent to the client
    let capturedEndpoint: string | undefined;
    let capturedPayload: any;
    mockClient.post = vi
      .fn()
      .mockImplementation((endpoint: string, payload: any) => {
        capturedEndpoint = endpoint;
        capturedPayload = payload;
        return Promise.resolve({ id: "test-tx-hash" });
      });

    const rollup = await createSolanaSignableRollup(
      {
        client: mockClient,
        getSerializer: () =>
          createMockSerializer({
            schema: { chain_data: { chain_id: 1, chain_name: "TestChain" } },
          }),
      },
      customEndpoint,
    );

    // Submit a Solana transaction to verify the custom endpoint is used
    await rollup.signAndSubmitTransaction(
      {
        runtime_call: { test: "call" },
        uniqueness: { generation: 123 },
        details: {
          max_priority_fee_bips: 0,
          max_fee: "1000",
          gas_limit: null,
          chain_id: 1,
        },
      } as any,
      {
        signer: createMockSigner(),
        authenticator: "solanaSimple",
      },
    );

    expect(capturedEndpoint).toBe(customEndpoint);
    expect(capturedPayload).toHaveProperty("body");
  });

  it("should read chain_name from schema.chain_data in fixture schema", async () => {
    const fixtureChainId = demoRollupSchema.chain_data.chain_id;
    const mockClient = createMockClient({ chainId: fixtureChainId });

    // Capture the payload sent to the client
    let capturedPayload: any;
    mockClient.post = vi
      .fn()
      .mockImplementation((path: string, options: any) => {
        capturedPayload = options;
        return Promise.resolve({ id: "test-tx-hash" });
      });

    mockClient.rollup.schema = vi.fn().mockResolvedValue({
      schema: demoRollupSchema,
      chain_hash:
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    });

    const rollup = await createSolanaSignableRollup({
      client: mockClient,
      getSerializer: (schema: any) => new JsSerializer(schema),
    });

    await rollup.signAndSubmitTransaction(
      {
        runtime_call: { test: "call" },
        uniqueness: { generation: 123 },
        details: {
          max_priority_fee_bips: 0,
          max_fee: "1000",
          gas_limit: null,
          chain_id: fixtureChainId,
        },
      } as any,
      {
        signer: createMockSigner(),
        authenticator: "solanaSimple",
      },
    );

    const bodyJson = JSON.parse(JSON.stringify(capturedPayload));
    const decodedBody = Buffer.from(bodyJson.body.body, "base64");
    const view = new DataView(
      decodedBody.buffer,
      decodedBody.byteOffset,
      decodedBody.byteLength,
    );
    const messageLength = view.getUint32(0, true);
    const jsonBytes = decodedBody.slice(4, 4 + messageLength);
    const message = JSON.parse(new TextDecoder().decode(jsonBytes));

    expect(message.chain_name).toBe(demoRollupSchema.chain_data.chain_name);
  });

  describe("byte-level compatibility with Rust implementation", () => {
    it("should generate identical bytes to Rust test_submit_raw_signed_message_transaction", async () => {
      // This test verifies that our TypeScript implementation generates the exact same bytes
      // as the Rust implementation

      // These values were generated using the test_submit_raw_signed_message_transaction() test from the sov-solana-offchain-auth crate.
      // The signer private key was logged, and the serde serialization of the AcceptTx was logged.
      const privateKeyHex =
        "4096e0037e7dc13c28730b01e303ea4679a05e019f68a5ee8aec6c1968cac707";
      const expectedJson =
        '{"body":{"body":"cAEAAHsicnVudGltZV9jYWxsIjp7ImJhbmsiOnsidHJhbnNmZXIiOnsidG8iOiI0emR3SE5hRWE1bnBIdFJ0YVozUkwxbTZycHR1UVo2UkJMSEc2Y0F5VkhqTCIsImNvaW5zIjp7ImFtb3VudCI6IjEwMDAwIiwidG9rZW5faWQiOiJ0b2tlbl8xbnlsMGUweXdlcmFnZnNhdHlndDI0em1kOGpycjJ2cXR2ZGZwdHpqaHhrZ3V6Mnh4eDN2czB5MDd1NyJ9fX19LCJ1bmlxdWVuZXNzIjp7ImdlbmVyYXRpb24iOjB9LCJkZXRhaWxzIjp7Im1heF9wcmlvcml0eV9mZWVfYmlwcyI6MCwibWF4X2ZlZSI6IjEwMDAwMDAwMDAwMCIsImdhc19saW1pdCI6WzEwMDAwMDAwMDAsMTAwMDAwMDAwMF0sImNoYWluX2lkIjo0MzIxfSwiY2hhaW5fbmFtZSI6IlRlc3RDaGFpbiJ9CwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwuMdhE5OjziHniAzu9qaFH0I50R93Apv2VgyONYPuLm3nN3Cr4cJwZ5ii6YYXxr7LsW3qcL0NAJfIvmUZroK+fuM18D3Hj+NsFn+nmN9jCjiWhjbQO1/79i365l424Erwg="}}';

      const mockClient = createMockClient({
        chainId: 4321,
        chainName: "TestChain",
        chainHash:
          "0x0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
      });

      // Capture the actual payload sent to the endpoint
      let capturedPayload: any;
      mockClient.post = vi
        .fn()
        .mockImplementation((path: string, options: any) => {
          capturedPayload = options;
          return Promise.resolve({ id: "test-tx-hash" });
        });

      const rollup = await createSolanaSignableRollup({
        client: mockClient,
        getSerializer: (schema: any) =>
          ({
            schema,
          }) as any,
      });

      const signer = new Ed25519Signer(privateKeyHex);

      // Transaction matching Rust test
      const runtimeCall = {
        bank: {
          transfer: {
            to: "4zdwHNaEa5npHtRtaZ3RL1m6rptuQZ6RBLHG6cAyVHjL",
            coins: {
              amount: "10000",
              token_id:
                "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7",
            },
          },
        },
      };

      const unsignedTx = {
        runtime_call: runtimeCall,
        uniqueness: { generation: 0 },
        details: {
          max_priority_fee_bips: 0,
          max_fee: "100000000000",
          gas_limit: [1000000000, 1000000000],
          chain_id: 4321,
        },
      };

      await rollup.signAndSubmitTransaction(unsignedTx, {
        signer,
        authenticator: "solanaSimple",
      });

      // Compare the entire POST body with the expected JSON from the Rust test
      const actualJson = JSON.stringify(capturedPayload);
      expect(actualJson).toBe(expectedJson);
    });

    it("should generate identical bytes to Rust test_ledger_signature_validation", async () => {
      // This test verifies that our TypeScript implementation generates the exact same bytes
      // as the Rust implementation for spec-compliant messages with preamble

      // These values will be generated from the test_ledger_signature_validation() test
      // from the sov-solana-offchain-auth crate
      const knownPubkeyHex =
        "70248c99a1d39769831c99706948b9851585cba907a677d112f9a3694cbcb4cd";
      const knownSignatureHex =
        "71204c3487b8e637cffaa5e9dc409efe2f1e98db6b557041181a368f4c487bbd407f72c43f08aa44b75f219e6fb3ce4681785dd72ee3eebe4e641ade8289370d";
      const expectedJson =
        '{"body":{"body":"xAEAAP9zb2xhbmEgb2ZmY2hhaW4ACwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsAAXAkjJmh05dpgxyZcGlIuYUVhcupB6Z30RL5o2lMvLTNbwF7InJ1bnRpbWVfY2FsbCI6eyJiYW5rIjp7InRyYW5zZmVyIjp7InRvIjoiNHpkd0hOYUVhNW5wSHRSdGFaM1JMMW02cnB0dVFaNlJCTEhHNmNBeVZIakwiLCJjb2lucyI6eyJhbW91bnQiOiI1MDAwIiwidG9rZW5faWQiOiJ0b2tlbl8xbnlsMGUweXdlcmFnZnNhdHlndDI0em1kOGpycjJ2cXR2ZGZwdHpqaHhrZ3V6Mnh4eDN2czB5MDd1NyJ9fX19LCJ1bmlxdWVuZXNzIjp7ImdlbmVyYXRpb24iOjB9LCJkZXRhaWxzIjp7Im1heF9wcmlvcml0eV9mZWVfYmlwcyI6MCwibWF4X2ZlZSI6IjEwMDAwMDAwMDAwMCIsImdhc19saW1pdCI6WzEwMDAwMDAwMDAsMTAwMDAwMDAwMF0sImNoYWluX2lkIjo0MzIxfSwiY2hhaW5fbmFtZSI6IlRlc3RDaGFpbiJ9cSBMNIe45jfP+qXp3ECe/i8emNtrVXBBGBo2j0xIe71Af3LEPwiqRLdfIZ5vs85GgXhd1y7j7r5OZBregok3DQ=="}}';

      const mockClient = createMockClient({
        chainId: 4321,
        chainName: "TestChain",
        chainHash:
          "0x0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
      });

      // Capture the actual payload sent to the endpoint
      let capturedPayload: any;
      mockClient.post = vi
        .fn()
        .mockImplementation((path: string, options: any) => {
          capturedPayload = options;
          return Promise.resolve({ id: "test-tx-hash" });
        });

      const rollup = await createSolanaSignableRollup({
        client: mockClient,
        getSerializer: (schema: any) =>
          ({
            schema,
          }) as any,
      });

      // Mock the signer to return the known public key and signature from Rust test
      const signer = {
        publicKey: vi
          .fn()
          .mockResolvedValue(
            new Uint8Array(Buffer.from(knownPubkeyHex, "hex")),
          ),
        sign: vi
          .fn()
          .mockResolvedValue(
            new Uint8Array(Buffer.from(knownSignatureHex, "hex")),
          ),
      } as any;

      // Transaction matching Rust test
      const runtimeCall = {
        bank: {
          transfer: {
            to: "4zdwHNaEa5npHtRtaZ3RL1m6rptuQZ6RBLHG6cAyVHjL",
            coins: {
              amount: "5000",
              token_id:
                "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7",
            },
          },
        },
      };

      const unsignedTx = {
        runtime_call: runtimeCall,
        uniqueness: { generation: 0 },
        details: {
          max_priority_fee_bips: 0,
          max_fee: "100000000000",
          gas_limit: [1000000000, 1000000000],
          chain_id: 4321,
        },
      };

      await rollup.signAndSubmitTransaction(unsignedTx, {
        signer,
        authenticator: "solana",
      });

      // Compare the entire POST body with the expected JSON from the Rust test
      const actualJson = JSON.stringify(capturedPayload);
      expect(actualJson).toBe(expectedJson);
    });
  });

  it("should generate identical bytes to Rust test_submit_multisig_simple_message_transaction", async () => {
    // These values were captured from the Rust test_submit_multisig_simple_message_transaction
    // integration test in sov-solana-offchain-auth.
    // The test uses a 2-of-3 multisig with signers 3 and 1 (out of order).
    const key1PrivHex =
      "09817894bf1e858df8d9bb3b931646c558ec4cacb9e4f9c05e91d0d788ec1142";
    const key2PrivHex =
      "71d81253990513758c7014bec174b4405988c133fc616d6f3d633170858f3dc9";
    const key3PrivHex =
      "90f1cca556a78435468bb17f116a923c8eb5c6074619a9bf39f28eb673a22a50";
    const expectedJson =
      '{"body":{"body":"swEAAIB7InJ1bnRpbWVfY2FsbCI6eyJiYW5rIjp7InRyYW5zZmVyIjp7InRvIjoiNHpkd0hOYUVhNW5wSHRSdGFaM1JMMW02cnB0dVFaNlJCTEhHNmNBeVZIakwiLCJjb2lucyI6eyJhbW91bnQiOiI3MDAwIiwidG9rZW5faWQiOiJ0b2tlbl8xbnlsMGUweXdlcmFnZnNhdHlndDI0em1kOGpycjJ2cXR2ZGZwdHpqaHhrZ3V6Mnh4eDN2czB5MDd1NyJ9fX19LCJ1bmlxdWVuZXNzIjp7Im5vbmNlIjowfSwiZGV0YWlscyI6eyJtYXhfcHJpb3JpdHlfZmVlX2JpcHMiOjAsIm1heF9mZWUiOiIxMDAwMDAwMDAwMDAiLCJnYXNfbGltaXQiOlsxMDAwMDAwMDAwLDEwMDAwMDAwMDBdLCJjaGFpbl9pZCI6NDMyMX0sImNoYWluX25hbWUiOiJUZXN0Q2hhaW4iLCJtdWx0aXNpZ19pZCI6Ino2RHlmUGVaekN4SkVEOFlBWTltQmRKcmpnYnBCWXFMVjh0TU1OcEt2M2siLCJ2ZXJzaW9uIjoxfQsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLAgAAALGwrShO91iJLqf6Ne0Bx0Mt4DCv/hQIhiR+LegEDayRgD14q6kXbVVUrAHhyhZG2TWlr+6W9OjwY8srNbqkXgcv4nipbIUhu2N6tX9gRlwQDXaiJqLt4WPbVSxip3aoIhQ6tjQZ1t9xE30vHWb2ATKwfZLkwlcd1YUR4NL/NyCTeNelR0QRS9XubQlHpFH6gWbr7vh/c84zN46qDBOR0woVYBc1xrVz6bzFCcgAxODD5kb1GRU3v+eRUbVRZ3udIAEAAAA1/Qt6TH3bXwUlsuG8tx6Fh26y7p57mnXqrGBSp+LzBQI="}}';

    const mockClient = createMockClient({
      chainId: 4321,
      chainName: "TestChain",
      chainHash:
        "0x0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
    });

    let capturedPayload: any;
    mockClient.post = vi
      .fn()
      .mockImplementation((path: string, options: any) => {
        capturedPayload = options;
        return Promise.resolve({ id: "test-tx-hash" });
      });

    const rollup = await createSolanaSignableRollup({
      client: mockClient,
      getSerializer: (schema: any) =>
        ({
          schema,
        }) as any,
    });

    const signer1 = new Ed25519Signer(key1PrivHex);
    const signer2 = new Ed25519Signer(key2PrivHex);
    const signer3 = new Ed25519Signer(key3PrivHex);

    // Compute the multisig address from the 3 public keys.
    // Mirrors MultisigTransaction.getMultisigAddress() from @sovereign-sdk/multisig.
    const pub1 = await signer1.publicKey();
    const pub2 = await signer2.publicKey();
    const pub3 = await signer3.publicKey();
    const pub1Hex = bytesToHex(pub1);
    const pub2Hex = bytesToHex(pub2);
    const pub3Hex = bytesToHex(pub3);
    const minSigners = 2;
    const multisigPubkeys = [pub1, pub2, pub3];

    const sortedPubKeys = [pub1Hex, pub2Hex, pub3Hex].sort();
    const pubKeyBytes = sortedPubKeys.map((pk) => Array.from(hexToBytes(pk)));
    const borshData = new Uint8Array(1 + 4 + 3 * 32);
    const dv = new DataView(borshData.buffer);
    borshData[0] = minSigners;
    dv.setUint32(1, 3, true);
    pubKeyBytes.forEach((pk, i) => borshData.set(pk, 5 + i * 32));
    const multisigAddress = sha256(borshData);

    const runtimeCall = {
      bank: {
        transfer: {
          to: "4zdwHNaEa5npHtRtaZ3RL1m6rptuQZ6RBLHG6cAyVHjL",
          coins: {
            amount: "7000",
            token_id:
              "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7",
          },
        },
      },
    };

    const unsignedTx = {
      runtime_call: runtimeCall,
      uniqueness: { nonce: 0 },
      details: {
        max_priority_fee_bips: 0,
        max_fee: "100000000000",
        gas_limit: [1000000000, 1000000000],
        chain_id: 4321,
      },
    };

    // Each signer signs independently (same order as Rust: key3, key1)
    const signedTx3 = await rollup.signTransactionForMultisig(unsignedTx, {
      signer: signer3,
      authenticator: "solanaSimple",
      multisigAddress,
      multisigPubkeys,
    });
    const signedTx1 = await rollup.signTransactionForMultisig(unsignedTx, {
      signer: signer1,
      authenticator: "solanaSimple",
      multisigAddress,
      multisigPubkeys,
    });

    // Build V1 transaction manually (avoids a cyclic dependency on @sovereign-sdk/multisig)
    const v0_3 = (signedTx3 as any).V0;
    const v0_1 = (signedTx1 as any).V0;

    const multisigV1 = {
      V1: {
        ...unsignedTx,
        signatures: [
          { pub_key: v0_3.pub_key, signature: v0_3.signature },
          { pub_key: v0_1.pub_key, signature: v0_1.signature },
        ],
        unused_pub_keys: [pub2Hex],
        min_signers: minSigners,
      },
    };

    await rollup.submitMultisigTransaction(multisigV1 as any, {
      authenticator: "solanaSimple",
      multisigAddress,
      multisigPubkeys,
    });

    const actualJson = JSON.stringify(capturedPayload);
    expect(actualJson).toBe(expectedJson);
  });

  it("should delegate standard multisig signing and submission to the inner rollup", async () => {
    const mockClient = createMockClient();

    let capturedEndpoint: string | undefined;
    mockClient.post = vi
      .fn()
      .mockImplementation((endpoint: string, options: any) => {
        capturedEndpoint = endpoint;
        return Promise.resolve({ id: "test-tx-hash" });
      });

    const rollup = await createSolanaSignableRollup({
      client: mockClient,
      getSerializer: () =>
        createMockSerializer({
          schema: { chain_data: { chain_id: 1, chain_name: "TestChain" } },
        }),
    });

    const signer = createMockSigner();
    const unsignedTx = {
      runtime_call: { test: "call" },
      uniqueness: { nonce: 0 },
      details: {
        max_priority_fee_bips: 0,
        max_fee: "1000",
        gas_limit: null,
        chain_id: 1,
      },
    };

    const signedTx = await rollup.signTransactionForMultisig(
      unsignedTx as any,
      {
        signer,
        authenticator: "standard",
      },
    );

    expect(signedTx).toHaveProperty("V0");
    expect(signer.sign).toHaveBeenCalledTimes(1);

    await rollup.submitMultisigTransaction(
      {
        V1: {
          ...unsignedTx,
          signatures: [],
          unused_pub_keys: [],
          min_signers: 0,
        },
      } as any,
      {
        authenticator: "standard",
      },
    );

    expect(capturedEndpoint).toBe("/sequencer/txs");
  });

  it("should match the Rust spec-compliant multisig request payload", async () => {
    const key1PrivHex =
      "d4ce78b7250da62754bd2b180aa95ecc63f2c79dd4a7cd1f104416e7039ae18b";
    const key2PrivHex =
      "1fdb54e03776d21349d68115151a29145ba89613dd7921604233f918b977c222";
    const key3PrivHex =
      "aa52d1811235c1c02cbbcf995b9dcabc7838a0931b12e97bf4eead7e0d573414";
    const expectedJson =
      '{"body":"SAIAAP9zb2xhbmEgb2ZmY2hhaW4ACwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsAAxCC8hWZvytLdhSuQz4hCg8AU5iG9i7Lm5d3TsrXXseqIq7fXUxXzPzWMSb2MrMb3ZHejz5ys9PwFhFBce+qogqkMLj7Z4EBZpuFaeJrH3G98UiHtBd00SHZoG/4/YYGvLMBeyJydW50aW1lX2NhbGwiOnsiYmFuayI6eyJ0cmFuc2ZlciI6eyJ0byI6IjR6ZHdITmFFYTVucEh0UnRhWjNSTDFtNnJwdHVRWjZSQkxIRzZjQXlWSGpMIiwiY29pbnMiOnsiYW1vdW50IjoiNzAwMCIsInRva2VuX2lkIjoidG9rZW5fMW55bDBlMHl3ZXJhZ2ZzYXR5Z3QyNHptZDhqcnIydnF0dmRmcHR6amh4a2d1ejJ4eHgzdnMweTA3dTcifX19fSwidW5pcXVlbmVzcyI6eyJub25jZSI6MH0sImRldGFpbHMiOnsibWF4X3ByaW9yaXR5X2ZlZV9iaXBzIjowLCJtYXhfZmVlIjoiMTAwMDAwMDAwMDAwIiwiZ2FzX2xpbWl0IjpbMTAwMDAwMDAwMCwxMDAwMDAwMDAwXSwiY2hhaW5faWQiOjQzMjF9LCJjaGFpbl9uYW1lIjoiVGVzdENoYWluIiwibXVsdGlzaWdfaWQiOiI2NFN2N2tMZVl0VXpVdGNuTTZCQVlqQXY4WjY1R2c1aXVtUGRVZzVaTXRKbiIsInZlcnNpb24iOjF9AgAAAAksFU/XcuxSQ2WBYJoZiYQf3gikgQi3CctMHuYBX3wBukIMQPhO5X7IwojPw5NtfjrbQhBVCSaLMjP7Bvi6SAsNM01gzyzMZ00ySPuO1RnF5Y0bTEDd57o8GWNCOHyizwqygFn1pehUMpBAp5FMeI1Fz7ZRe7K7mHiu26MFwq8HBgAAAAI="}';

    const mockClient = createMockClient({
      chainId: 4321,
      chainName: "TestChain",
      chainHash:
        "0x0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
    });

    let capturedPayload: any;
    mockClient.post = vi
      .fn()
      .mockImplementation((path: string, options: any) => {
        capturedPayload = options;
        return Promise.resolve({ id: "test-tx-hash" });
      });

    const rollup = await createSolanaSignableRollup({
      client: mockClient,
      getSerializer: (schema: any) =>
        ({
          schema,
        }) as any,
    });

    const signer1 = new Ed25519Signer(key1PrivHex);
    const signer2 = new Ed25519Signer(key2PrivHex);
    const signer3 = new Ed25519Signer(key3PrivHex);
    const pub1 = await signer1.publicKey();
    const pub2 = await signer2.publicKey();
    const pub3 = await signer3.publicKey();
    const pub1Hex = bytesToHex(pub1);
    const pub2Hex = bytesToHex(pub2);
    const pub3Hex = bytesToHex(pub3);
    const minSigners = 2;
    const multisigPubkeys = [pub1, pub2, pub3];

    const sortedPubKeys = [pub1Hex, pub2Hex, pub3Hex].sort();
    const pubKeyBytes = sortedPubKeys.map((pk) => Array.from(hexToBytes(pk)));
    const borshData = new Uint8Array(1 + 4 + 3 * 32);
    const dv = new DataView(borshData.buffer);
    borshData[0] = minSigners;
    dv.setUint32(1, 3, true);
    pubKeyBytes.forEach((pk, i) => borshData.set(pk, 5 + i * 32));
    const multisigAddress = sha256(borshData);

    const unsignedTx = {
      runtime_call: {
        bank: {
          transfer: {
            to: "4zdwHNaEa5npHtRtaZ3RL1m6rptuQZ6RBLHG6cAyVHjL",
            coins: {
              amount: "7000",
              token_id:
                "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7",
            },
          },
        },
      },
      uniqueness: { nonce: 0 },
      details: {
        max_priority_fee_bips: 0,
        max_fee: "100000000000",
        gas_limit: [1000000000, 1000000000],
        chain_id: 4321,
      },
    };

    const signedTx3 = await rollup.signTransactionForMultisig(unsignedTx, {
      signer: signer3,
      authenticator: "solana",
      multisigAddress,
      multisigPubkeys,
    });
    const signedTx1 = await rollup.signTransactionForMultisig(unsignedTx, {
      signer: signer1,
      authenticator: "solana",
      multisigAddress,
      multisigPubkeys,
    });

    const v0_3 = (signedTx3 as any).V0;
    const v0_1 = (signedTx1 as any).V0;

    const multisigV1 = {
      V1: {
        ...unsignedTx,
        signatures: [
          { pub_key: v0_3.pub_key, signature: v0_3.signature },
          { pub_key: v0_1.pub_key, signature: v0_1.signature },
        ],
        unused_pub_keys: [pub2Hex],
        min_signers: minSigners,
      },
    };

    await rollup.submitMultisigTransaction(multisigV1 as any, {
      authenticator: "solana",
      multisigAddress,
      multisigPubkeys,
    });

    const actualJson = JSON.stringify(capturedPayload.body);
    expect(actualJson).toBe(expectedJson);
  });

  describe("solanaAuto authenticator", () => {
    it("should use 'solana' authenticator for LedgerSolanaSigner", async () => {
      const mockClient = createMockClient();

      // Capture the actual payload sent to the endpoint
      let capturedPayload: any;
      mockClient.post = vi
        .fn()
        .mockImplementation((path: string, options: any) => {
          capturedPayload = options;
          return Promise.resolve({ id: "test-tx-hash" });
        });

      const rollup = await createSolanaSignableRollup({
        client: mockClient,
        getSerializer: () =>
          createMockSerializer({
            schema: { chain_data: { chain_id: 1, chain_name: "TestChain" } },
          }),
      });

      // Create a real LedgerSolanaSigner instance and mock its methods
      const ledgerSigner = new LedgerSolanaSigner();
      vi.spyOn(ledgerSigner, "publicKey").mockResolvedValue(new Uint8Array(32));
      vi.spyOn(ledgerSigner, "sign").mockResolvedValue(new Uint8Array(64));

      await rollup.signAndSubmitTransaction(
        {
          runtime_call: { test: "call" },
          uniqueness: { generation: 123 },
          details: {
            max_priority_fee_bips: 0,
            max_fee: "1000",
            gas_limit: null,
            chain_id: 1,
          },
        } as any,
        {
          signer: ledgerSigner,
          authenticator: "solanaAuto",
        },
      );

      // Verify that spec-compliant message was sent (it will have the preamble)
      const bodyJson = JSON.parse(JSON.stringify(capturedPayload));
      const decodedBody = Buffer.from(bodyJson.body.body, "base64");

      // Check for Solana offchain signing domain in preamble
      // The spec-compliant message has a 4-byte length prefix, then the preamble starts with 0xff
      expect(decodedBody[4]).toBe(0xff); // Skip 4-byte length prefix
      const signingDomain = new TextDecoder().decode(decodedBody.slice(5, 20)); // 4 + 1 + 15 = 20
      expect(signingDomain).toBe("solana offchain");
    });

    it("should use 'solanaSimple' authenticator for Ed25519Signer", async () => {
      const mockClient = createMockClient();

      // Capture the actual payload sent to the endpoint
      let capturedPayload: any;
      mockClient.post = vi
        .fn()
        .mockImplementation((path: string, options: any) => {
          capturedPayload = options;
          return Promise.resolve({ id: "test-tx-hash" });
        });

      const rollup = await createSolanaSignableRollup({
        client: mockClient,
        getSerializer: () =>
          createMockSerializer({
            schema: { chain_data: { chain_id: 1, chain_name: "TestChain" } },
          }),
      });

      // Create a regular Ed25519Signer
      const ed25519Signer = createMockSigner();

      await rollup.signAndSubmitTransaction(
        {
          runtime_call: { test: "call" },
          uniqueness: { generation: 123 },
          details: {
            max_priority_fee_bips: 0,
            max_fee: "1000",
            gas_limit: null,
            chain_id: 1,
          },
        } as any,
        {
          signer: ed25519Signer,
          authenticator: "solanaAuto",
        },
      );

      // Verify that simple message was sent (no preamble)
      const bodyJson = JSON.parse(JSON.stringify(capturedPayload));
      const decodedBody = Buffer.from(bodyJson.body.body, "base64");

      // Simple message starts with a length prefix (4 bytes) followed by the JSON message
      // It should NOT have the 0xff signing domain marker
      expect(decodedBody[0]).not.toBe(0xff);
    });
  });
});
