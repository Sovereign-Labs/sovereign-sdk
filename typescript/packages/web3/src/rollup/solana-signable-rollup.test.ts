import SovereignClient from "@sovereign-sdk/client";
import { Multisig } from "@sovereign-sdk/multisig";
import { JsSerializer } from "@sovereign-sdk/serializers";
import { Ed25519Signer } from "@sovereign-sdk/signers";
import { bytesToHex } from "@sovereign-sdk/utils";
import bs58 from "bs58";
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
    serializeSigningPayload: vi.fn().mockReturnValue(new Uint8Array(10)),
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
          chain_hash_fragment: "42",
          max_priority_fee_bips: 100,
          max_fee: "200000000",
          gas_limit: null,
        },
      },
    };

    const rollup = await createSolanaSignableRollup(customConfig);

    expect(rollup.context.defaultTxDetails.chain_hash_fragment).toBe("42");
    expect(rollup.context.defaultTxDetails.max_priority_fee_bips).toBe(100);
    expect(rollup.context.defaultTxDetails.max_fee).toBe("200000000");
  });

  it("should work without any configuration", async () => {
    const mockClient = createMockClient();

    const rollup = await createSolanaSignableRollup({ client: mockClient });

    expect(rollup).toBeInstanceOf(SolanaSignableRollup);
    expect(rollup.context.defaultTxDetails.chain_hash_fragment).toBe("0");
  });

  it("should allow custom Solana endpoint configuration", async () => {
    const mockClient = createMockClient();
    const customEndpoint = "/custom/solana-tx-endpoint";
    const requestOptions = {
      timeout: 1234,
      headers: { "x-test-header": "solana" },
    };

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
          chain_hash_fragment: "0",
        },
      } as any,
      {
        signer: createMockSigner(),
        authenticator: "solanaSimple",
      },
      requestOptions,
    );

    expect(capturedEndpoint).toBe(customEndpoint);
    expect(capturedPayload).toHaveProperty("body");
    expect(capturedPayload.timeout).toBe(requestOptions.timeout);
    expect(capturedPayload.headers).toEqual(requestOptions.headers);
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
          chain_hash_fragment: "0",
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
    expect(message.version).toBe(0);
  });

  it("should include address_override in Solana signed JSON when present", async () => {
    const mockClient = createMockClient({ chainName: "TestChain" });
    let capturedPayload: any;
    mockClient.post = vi
      .fn()
      .mockImplementation((_path: string, options: any) => {
        capturedPayload = options;
        return Promise.resolve({ id: "test-tx-hash" });
      });

    const rollup = await createSolanaSignableRollup({
      client: mockClient,
      getSerializer: (schema: any) => ({ schema }) as any,
    });

    await rollup.signAndSubmitTransaction(
      {
        runtime_call: { test: "call" },
        uniqueness: { generation: 123 },
        details: {
          max_priority_fee_bips: 0,
          max_fee: "1000",
          gas_limit: null,
          chain_hash_fragment: "0",
        },
        address_override: "sov1target",
      },
      {
        signer: createMockSigner(),
        authenticator: "solanaSimple",
      },
    );

    const decodedBody = Buffer.from(capturedPayload.body.body, "base64");
    const view = new DataView(
      decodedBody.buffer,
      decodedBody.byteOffset,
      decodedBody.byteLength,
    );
    const messageLength = view.getUint32(0, true);
    const jsonBytes = decodedBody.slice(4, 4 + messageLength);
    const message = JSON.parse(new TextDecoder().decode(jsonBytes));

    expect(message.address_override).toBe("sov1target");
  });

  it("should refresh stale chain hash fragments before Solana simple signing", async () => {
    const mockClient = createMockClient({
      chainName: "TestChain",
      chainHash:
        "0x0102030405060708000000000000000000000000000000000000000000000000",
    });
    let capturedPayload: any;
    mockClient.post = vi
      .fn()
      .mockImplementation((_path: string, options: any) => {
        capturedPayload = options;
        return Promise.resolve({ id: "test-tx-hash" });
      });

    const rollup = await createSolanaSignableRollup({
      client: mockClient,
      getSerializer: (schema: any) => ({ schema }) as any,
    });

    const unsignedTx = {
      runtime_call: { test: "call" },
      uniqueness: { generation: 123 },
      details: {
        max_priority_fee_bips: 0,
        max_fee: "1000",
        gas_limit: null,
        chain_hash_fragment: "stale",
      },
      address_override: null,
    };

    await rollup.signAndSubmitTransaction(unsignedTx, {
      signer: createMockSigner(),
      authenticator: "solanaSimple",
    });

    const decodedBody = Buffer.from(capturedPayload.body.body, "base64");
    const view = new DataView(
      decodedBody.buffer,
      decodedBody.byteOffset,
      decodedBody.byteLength,
    );
    const messageLength = view.getUint32(0, true);
    const jsonBytes = decodedBody.slice(4, 4 + messageLength);
    const message = JSON.parse(new TextDecoder().decode(jsonBytes));

    expect(message.details.chain_hash_fragment).toBe("578437695752307201");
    expect(unsignedTx.details.chain_hash_fragment).toBe("578437695752307201");
  });

  describe("byte-level compatibility with Rust implementation", () => {
    it("should generate identical bytes to Rust test_submit_raw_signed_message_transaction", async () => {
      // This test verifies that our TypeScript implementation generates the exact same bytes
      // as the Rust implementation

      // These values were generated using the test_submit_raw_signed_message_transaction() test from the sov-solana-offchain-auth crate.
      // The signer private key was logged, and the serde serialization of the AcceptTx was logged.
      const privateKeyHex =
        "2bf7a34f197040d49014e026aa35a61b094ad6b32d5ae86e769e777a51a83c5d";
      const expectedJson =
        '{"body":{"body":"lwEAAHsicnVudGltZV9jYWxsIjp7ImJhbmsiOnsidHJhbnNmZXIiOnsidG8iOiI0emR3SE5hRWE1bnBIdFJ0YVozUkwxbTZycHR1UVo2UkJMSEc2Y0F5VkhqTCIsImNvaW5zIjp7ImFtb3VudCI6IjEwMDAwIiwidG9rZW5faWQiOiJ0b2tlbl8xbnlsMGUweXdlcmFnZnNhdHlndDI0em1kOGpycjJ2cXR2ZGZwdHpqaHhrZ3V6Mnh4eDN2czB5MDd1NyJ9fX19LCJ1bmlxdWVuZXNzIjp7ImdlbmVyYXRpb24iOjB9LCJkZXRhaWxzIjp7Im1heF9wcmlvcml0eV9mZWVfYmlwcyI6MCwibWF4X2ZlZSI6IjEwMDAwMDAwMDAwMCIsImdhc19saW1pdCI6WzEwMDAwMDAwMDAsMTAwMDAwMDAwMF0sImNoYWluX2hhc2hfZnJhZ21lbnQiOiI3OTU3NDE5MDEyMTg4NDM0MDMifSwiY2hhaW5fbmFtZSI6IlRlc3RDaGFpbiIsInZlcnNpb24iOjB9CwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwt0I7oBLzTClR8+NiHHPE6LyyQfcVTjHex79N5keSD1gsvqIJOZp17q3OiVs8imQ1uYVbpGTr2eKDQZzAC9OEWF5Q0+fWsBtlViK2F9L6NO38U/7NknbOyhuXUZxh7pMQs="}}';

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
          chain_hash_fragment: "795741901218843403",
        },
        address_override: null,
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
        "89941ccebc40db1daf60b0b392121616868000d4799d930d18f4eaff266cd560cf61c6bf62c97955f64b33cc0640718427d0dc0180cf65e9746b7522d7f6d20e";
      const expectedJson =
        '{"body":{"body":"6wEAAP9zb2xhbmEgb2ZmY2hhaW4ACwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsAAXAkjJmh05dpgxyZcGlIuYUVhcupB6Z30RL5o2lMvLTNlgF7InJ1bnRpbWVfY2FsbCI6eyJiYW5rIjp7InRyYW5zZmVyIjp7InRvIjoiNHpkd0hOYUVhNW5wSHRSdGFaM1JMMW02cnB0dVFaNlJCTEhHNmNBeVZIakwiLCJjb2lucyI6eyJhbW91bnQiOiI1MDAwIiwidG9rZW5faWQiOiJ0b2tlbl8xbnlsMGUweXdlcmFnZnNhdHlndDI0em1kOGpycjJ2cXR2ZGZwdHpqaHhrZ3V6Mnh4eDN2czB5MDd1NyJ9fX19LCJ1bmlxdWVuZXNzIjp7ImdlbmVyYXRpb24iOjB9LCJkZXRhaWxzIjp7Im1heF9wcmlvcml0eV9mZWVfYmlwcyI6MCwibWF4X2ZlZSI6IjEwMDAwMDAwMDAwMCIsImdhc19saW1pdCI6WzEwMDAwMDAwMDAsMTAwMDAwMDAwMF0sImNoYWluX2hhc2hfZnJhZ21lbnQiOiI3OTU3NDE5MDEyMTg4NDM0MDMifSwiY2hhaW5fbmFtZSI6IlRlc3RDaGFpbiIsInZlcnNpb24iOjB9iZQczrxA2x2vYLCzkhIWFoaAANR5nZMNGPTq/yZs1WDPYca/Ysl5VfZLM8wGQHGEJ9DcAYDPZel0a3Ui1/bSDg=="}}';

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
          chain_hash_fragment: "795741901218843403",
        },
        address_override: null,
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
      '{"body":{"body":"zgEAAIB7InJ1bnRpbWVfY2FsbCI6eyJiYW5rIjp7InRyYW5zZmVyIjp7InRvIjoiNHpkd0hOYUVhNW5wSHRSdGFaM1JMMW02cnB0dVFaNlJCTEhHNmNBeVZIakwiLCJjb2lucyI6eyJhbW91bnQiOiI3MDAwIiwidG9rZW5faWQiOiJ0b2tlbl8xbnlsMGUweXdlcmFnZnNhdHlndDI0em1kOGpycjJ2cXR2ZGZwdHpqaHhrZ3V6Mnh4eDN2czB5MDd1NyJ9fX19LCJ1bmlxdWVuZXNzIjp7Im5vbmNlIjowfSwiZGV0YWlscyI6eyJtYXhfcHJpb3JpdHlfZmVlX2JpcHMiOjAsIm1heF9mZWUiOiIxMDAwMDAwMDAwMDAiLCJnYXNfbGltaXQiOlsxMDAwMDAwMDAwLDEwMDAwMDAwMDBdLCJjaGFpbl9oYXNoX2ZyYWdtZW50IjoiNzk1NzQxOTAxMjE4ODQzNDAzIn0sImNoYWluX25hbWUiOiJUZXN0Q2hhaW4iLCJtdWx0aXNpZ19pZCI6Ino2RHlmUGVaekN4SkVEOFlBWTltQmRKcmpnYnBCWXFMVjh0TU1OcEt2M2siLCJ2ZXJzaW9uIjoxfQsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLAgAAAJ/0JLjGX1H2lXqpOX6Cdm7hhLtiGbUOtvVb9sEj9bSCWfS5gD2U+yGng1skqC5EslHi10tXa34gvGw67B3EgQIv4nipbIUhu2N6tX9gRlwQDXaiJqLt4WPbVSxip3aoIri6+DTe2fz0jV+E9R1N1Myb+psGvw8oB3UR5Pq1ScpyiXqAwUd2FO2ecZv//lhu2cCWqzzPhburzCIw11tnQg4VYBc1xrVz6bzFCcgAxODD5kb1GRU3v+eRUbVRZ3udIAEAAAA1/Qt6TH3bXwUlsuG8tx6Fh26y7p57mnXqrGBSp+LzBQI="}}';

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
    const multisig = Multisig.fromPubKeys(
      [pub1Hex, pub2Hex, pub3Hex],
      minSigners,
    );

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
        chain_hash_fragment: "795741901218843403",
      },
      address_override: null,
    };

    const signer3Bytes = await rollup.multisigSigningBytes(
      unsignedTx,
      multisig,
      "solanaSimple",
    );
    multisig.addSignature(
      bytesToHex(await signer3.sign(signer3Bytes)),
      bytesToHex(await signer3.publicKey()),
    );
    const signer1Bytes = await rollup.multisigSigningBytes(
      unsignedTx,
      multisig,
      "solanaSimple",
    );
    multisig.addSignature(
      bytesToHex(await signer1.sign(signer1Bytes)),
      bytesToHex(await signer1.publicKey()),
    );
    const multisigV1 = multisig.toTransaction(unsignedTx);

    await rollup.submitTransaction(multisigV1 as any, "solanaSimple");

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
        chain_hash_fragment: "0",
      },
      address_override: null,
    };

    const pubKeyHex = bytesToHex(await signer.publicKey());
    const otherPubKeyHex = bytesToHex(new Uint8Array(32).fill(9));
    const multisig = Multisig.fromPubKeys([pubKeyHex, otherPubKeyHex], 1);

    const signingBytes = await rollup.multisigSigningBytes(
      unsignedTx as any,
      multisig,
      "standard",
    );
    multisig.addSignature(
      bytesToHex(await signer.sign(signingBytes)),
      bytesToHex(await signer.publicKey()),
    );

    expect(multisig.signaturesAndPubKeys).toHaveLength(1);
    expect(signer.sign).toHaveBeenCalledTimes(1);

    await rollup.submitTransaction(
      multisig.toTransaction(unsignedTx as any),
      "standard",
    );

    expect(capturedEndpoint).toBe("/sequencer/txs");
  });

  it.each(["solanaSimple", "solana"] as const)(
    "should enforce the fragment invariant without mutation for %s multisig signing",
    async (authenticator) => {
      const mockClient = createMockClient({
        chainName: "TestChain",
        chainHash:
          "0x0102030405060708000000000000000000000000000000000000000000000000",
      });
      const rollup = await createSolanaSignableRollup({
        client: mockClient,
        getSerializer: () =>
          createMockSerializer({
            schema: { chain_data: { chain_name: "TestChain" } },
          }),
      });
      const multisig = Multisig.fromPubKeys(
        [
          bytesToHex(new Uint8Array(32).fill(1)),
          bytesToHex(new Uint8Array(32).fill(2)),
        ],
        1,
      );
      const unsignedTx = {
        runtime_call: { test: "call" },
        uniqueness: { nonce: 0 },
        details: {
          max_priority_fee_bips: 0,
          max_fee: "1000",
          gas_limit: null,
          chain_hash_fragment: "578437695752307201",
        },
        address_override: null,
      };
      const originalDetails = unsignedTx.details;
      const originalUnsignedTx = {
        ...unsignedTx,
        details: { ...unsignedTx.details },
      };

      await expect(
        rollup.multisigSigningBytes(unsignedTx, multisig, authenticator),
      ).resolves.toBeInstanceOf(Uint8Array);
      expect(unsignedTx).toEqual(originalUnsignedTx);
      expect(unsignedTx.details).toBe(originalDetails);

      const staleUnsignedTx = {
        ...unsignedTx,
        details: {
          ...unsignedTx.details,
          chain_hash_fragment: "stale",
        },
      };
      const originalStaleDetails = staleUnsignedTx.details;
      const originalStaleUnsignedTx = {
        ...staleUnsignedTx,
        details: { ...staleUnsignedTx.details },
      };

      await expect(
        rollup.multisigSigningBytes(staleUnsignedTx, multisig, authenticator),
      ).rejects.toThrow(
        "Cannot sign multisig transaction: chain_hash_fragment stale does not match the current chain hash fragment 578437695752307201",
      );
      expect(staleUnsignedTx).toEqual(originalStaleUnsignedTx);
      expect(staleUnsignedTx.details).toBe(originalStaleDetails);
    },
  );

  it("should match the Rust spec-compliant multisig request payload", async () => {
    const key1PrivHex =
      "d4ce78b7250da62754bd2b180aa95ecc63f2c79dd4a7cd1f104416e7039ae18b";
    const key2PrivHex =
      "1fdb54e03776d21349d68115151a29145ba89613dd7921604233f918b977c222";
    const key3PrivHex =
      "aa52d1811235c1c02cbbcf995b9dcabc7838a0931b12e97bf4eead7e0d573414";
    const expectedJson =
      '{"body":"YwIAAP9zb2xhbmEgb2ZmY2hhaW4ACwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsAAxCC8hWZvytLdhSuQz4hCg8AU5iG9i7Lm5d3TsrXXseqIq7fXUxXzPzWMSb2MrMb3ZHejz5ys9PwFhFBce+qogqkMLj7Z4EBZpuFaeJrH3G98UiHtBd00SHZoG/4/YYGvM4BeyJydW50aW1lX2NhbGwiOnsiYmFuayI6eyJ0cmFuc2ZlciI6eyJ0byI6IjR6ZHdITmFFYTVucEh0UnRhWjNSTDFtNnJwdHVRWjZSQkxIRzZjQXlWSGpMIiwiY29pbnMiOnsiYW1vdW50IjoiNzAwMCIsInRva2VuX2lkIjoidG9rZW5fMW55bDBlMHl3ZXJhZ2ZzYXR5Z3QyNHptZDhqcnIydnF0dmRmcHR6amh4a2d1ejJ4eHgzdnMweTA3dTcifX19fSwidW5pcXVlbmVzcyI6eyJub25jZSI6MH0sImRldGFpbHMiOnsibWF4X3ByaW9yaXR5X2ZlZV9iaXBzIjowLCJtYXhfZmVlIjoiMTAwMDAwMDAwMDAwIiwiZ2FzX2xpbWl0IjpbMTAwMDAwMDAwMCwxMDAwMDAwMDAwXSwiY2hhaW5faGFzaF9mcmFnbWVudCI6Ijc5NTc0MTkwMTIxODg0MzQwMyJ9LCJjaGFpbl9uYW1lIjoiVGVzdENoYWluIiwibXVsdGlzaWdfaWQiOiI2NFN2N2tMZVl0VXpVdGNuTTZCQVlqQXY4WjY1R2c1aXVtUGRVZzVaTXRKbiIsInZlcnNpb24iOjF9AgAAALqZ6CBwaWxjUG2qZcmZzTQxJ9d0+NzYgNS7g2391KSpnEq+Pcxj/YG3G9tkg4jaSXz+RgHWwgCPAbXNcnKbDQUnH8ulC3nADh3qr0/Xf9X7VRaQbgPm7NGJI6AhWW7p2aRy07HkyaoueAVpYzYhiaai7zl13usxx04dmlqkSMEJBgAAAAI="}';

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
    const multisig = Multisig.fromPubKeys(
      [pub1Hex, pub2Hex, pub3Hex],
      minSigners,
    );

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
        chain_hash_fragment: "795741901218843403",
      },
      address_override: null,
    };

    const signer3Bytes = await rollup.multisigSigningBytes(
      unsignedTx,
      multisig,
      "solana",
    );
    multisig.addSignature(
      bytesToHex(await signer3.sign(signer3Bytes)),
      bytesToHex(await signer3.publicKey()),
    );
    const signer1Bytes = await rollup.multisigSigningBytes(
      unsignedTx,
      multisig,
      "solana",
    );
    multisig.addSignature(
      bytesToHex(await signer1.sign(signer1Bytes)),
      bytesToHex(await signer1.publicKey()),
    );
    const multisigV1 = multisig.toTransaction(unsignedTx);

    await rollup.submitTransaction(multisigV1 as any, "solana");

    const actualJson = JSON.stringify(capturedPayload.body);
    expect(actualJson).toBe(expectedJson);
  });

  describe("V1 address_override", () => {
    async function setupMultisigContext(): Promise<{
      rollup: SolanaSignableRollup<unknown>;
      capturedPayloadRef: { current: any };
      multisig: Multisig;
      signers: {
        signer1: Ed25519Signer;
        signer2: Ed25519Signer;
        signer3: Ed25519Signer;
      };
      unsignedTx: any;
    }> {
      const key1PrivHex =
        "09817894bf1e858df8d9bb3b931646c558ec4cacb9e4f9c05e91d0d788ec1142";
      const key2PrivHex =
        "71d81253990513758c7014bec174b4405988c133fc616d6f3d633170858f3dc9";
      const key3PrivHex =
        "90f1cca556a78435468bb17f116a923c8eb5c6074619a9bf39f28eb673a22a50";
      const mockClient = createMockClient({
        chainId: 4321,
        chainName: "TestChain",
        chainHash:
          "0x0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
      });
      const capturedPayloadRef: { current: any } = { current: undefined };
      mockClient.post = vi
        .fn()
        .mockImplementation((_path: string, options: any) => {
          capturedPayloadRef.current = options;
          return Promise.resolve({ id: "test-tx-hash" });
        });

      const rollup = await createSolanaSignableRollup({
        client: mockClient,
        getSerializer: (schema: any) => ({ schema }) as any,
      });
      const signer1 = new Ed25519Signer(key1PrivHex);
      const signer2 = new Ed25519Signer(key2PrivHex);
      const signer3 = new Ed25519Signer(key3PrivHex);
      const multisig = Multisig.fromPubKeys(
        [
          bytesToHex(await signer1.publicKey()),
          bytesToHex(await signer2.publicKey()),
          bytesToHex(await signer3.publicKey()),
        ],
        2,
      );
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
          chain_hash_fragment: "795741901218843403",
        },
        address_override: null,
      };

      return {
        rollup,
        capturedPayloadRef,
        multisig,
        signers: { signer1, signer2, signer3 },
        unsignedTx,
      };
    }

    async function signMultisig(
      rollup: SolanaSignableRollup<unknown>,
      unsignedTx: any,
      multisig: Multisig,
      signer: Ed25519Signer,
    ): Promise<void> {
      const signingBytes = await rollup.multisigSigningBytes(
        unsignedTx,
        multisig,
        "solanaSimple",
      );
      multisig.addSignature(
        bytesToHex(await signer.sign(signingBytes)),
        bytesToHex(await signer.publicKey()),
      );
    }

    it("omits address_override from multisig signing bytes when it is null", async () => {
      const { rollup, multisig, unsignedTx } = await setupMultisigContext();

      const signingBytes = await rollup.multisigSigningBytes(
        unsignedTx,
        multisig,
        "solanaSimple",
      );
      const signedJson = new TextDecoder().decode(signingBytes);

      expect(JSON.parse(signedJson)).not.toHaveProperty("address_override");
    });

    it("includes tx.address_override in multisig signing bytes before version", async () => {
      const { rollup, multisig, unsignedTx } = await setupMultisigContext();
      const addressOverride = new Uint8Array(32).fill(0x42);
      const addressOverrideBs58 = bs58.encode(addressOverride);

      const signingBytes = await rollup.multisigSigningBytes(
        { ...unsignedTx, address_override: addressOverrideBs58 },
        multisig,
        "solanaSimple",
      );
      const signedJson = new TextDecoder().decode(signingBytes);

      expect(JSON.parse(signedJson).address_override).toBe(addressOverrideBs58);
      expect(signedJson.indexOf('"address_override"')).toBeLessThan(
        signedJson.indexOf('"version"'),
      );
    });

    it("forwards tx.address_override into submitted multisig JSON", async () => {
      const { rollup, capturedPayloadRef, multisig, signers, unsignedTx } =
        await setupMultisigContext();
      const addressOverride = new Uint8Array(32).fill(0x99);
      const addressOverrideBs58 = bs58.encode(addressOverride);
      const txWithOverride = {
        ...unsignedTx,
        address_override: addressOverrideBs58,
      };

      await signMultisig(rollup, txWithOverride, multisig, signers.signer3);
      await signMultisig(rollup, txWithOverride, multisig, signers.signer1);

      await rollup.submitTransaction(
        multisig.toTransaction(txWithOverride),
        "solanaSimple",
      );

      const submittedBody = capturedPayloadRef.current.body.body as string;
      const submittedText = new TextDecoder().decode(
        Buffer.from(submittedBody, "base64"),
      );
      expect(submittedText).toContain(
        `"address_override":"${addressOverrideBs58}"`,
      );
    });
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

      const ledgerSigner = {
        __ledgerSolanaSigner: true as const,
        publicKey: vi.fn().mockResolvedValue(new Uint8Array(32)),
        sign: vi.fn().mockResolvedValue(new Uint8Array(64)),
      };

      await rollup.signAndSubmitTransaction(
        {
          runtime_call: { test: "call" },
          uniqueness: { generation: 123 },
          details: {
            max_priority_fee_bips: 0,
            max_fee: "1000",
            gas_limit: null,
            chain_hash_fragment: "0",
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
            chain_hash_fragment: "0",
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
