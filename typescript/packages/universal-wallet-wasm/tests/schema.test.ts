import { KnownTypeId, Schema } from "@sovereign-sdk/universal-wallet-wasm";
import { describe, expect, it } from "vitest";
import demoRollupSchema from "../../__fixtures__/demo-rollup-schema.json";
import { bytesToHex, hexToBytes } from "./utils";

const schema = Schema.fromJSON(JSON.stringify(demoRollupSchema));

describe("Schema", () => {
  describe("fromJSON", () => {
    it("should give descriptive error on invalid schema", () => {
      let err: Error;

      try {
        Schema.fromJSON("{}");
      } catch (e) {
        err = e as Error;
      }

      expect(err!).toBeInstanceOf(Error);
      expect(err!.message).toMatch(/missing field `types`/);
    });
  });
  describe("descriptor", () => {
    it("should return the descriptor used to create the schema", () => {
      const expected = JSON.stringify(demoRollupSchema);

      expect(schema.descriptor).toEqual(expected);
    });
  });
  describe("chainHash", () => {
    it("should calculate the chain hash successfully", () => {
      const expected =
        "10c4d33e43ee94d78fba9ed555bcc65b75c03fefe27b3ba862a92ea3299d09fc";
      const actual = bytesToHex(schema.chainHash);

      expect(actual).toEqual(expected);
    });
  });
  describe("metadataHash", () => {
    it("should restore the metadata hash successfully", () => {
      const expected =
        "814bfec8f2bcef818e22e6981423a702633d5cd1e29cd362757ce24a1397e6ac";
      const actual = bytesToHex(schema.metadataHash);

      expect(actual).toEqual(expected);
    });
  });
  describe("jsonToBorsh", () => {
    it("should serialize a simple json object to borsh", () => {
      const call = {
        bank: {
          create_token: {
            token_name: "token_1",
            initial_balance: "20000",
            token_decimals: 12,
            supply_cap: "100000000000",
            mint_to_address: {
              Standard:
                "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
            },
            admins: [
              {
                Standard:
                  "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
              },
            ],
          },
        },
      };
      const actual = bytesToHex(
        schema.jsonToBorsh(
          schema.knownTypeIndex(KnownTypeId.RuntimeCall),
          JSON.stringify(call),
        ),
      );
      const expected =
        "000007000000746f6b656e5f31010c204e000000000000000000000000000000f8ad2437a279e1c8932c07358c91dc4fe34864a98c6c25f298e2a0190100000000f8ad2437a279e1c8932c07358c91dc4fe34864a98c6c25f298e2a0190100e87648170000000000000000000000";

      expect(actual).toEqual(expected);
    });
    it("should return concise and useful error messages", () => {
      const call = {
        bank: {
          create_token: {},
        },
      };
      const doConversion = () =>
        schema.jsonToBorsh(
          schema.knownTypeIndex(KnownTypeId.RuntimeCall),
          JSON.stringify(call),
        );
      expect(doConversion).toThrow(
        "Expected type or field __SovVirtualWallet_CallMessage_CreateToken.token_name, but it was not present",
      );
    });
    it("should allow strings to serialize as u128", () => {
      const addr = hexToBytes(
        "b7e23f9dc86a1547ee09d82a5c8f3610d975e2c84fb61038a719e524",
      );
      const call = {
        bank: {
          transfer: {
            to: { Standard: Array.from(addr) },
            coins: {
              amount: "110000000000000000000000000000000091337",
              token_id:
                "token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6",
            },
          },
        },
      };
      const doConversion = () =>
        schema.jsonToBorsh(
          schema.knownTypeIndex(KnownTypeId.RuntimeCall),
          JSON.stringify(call),
        );
      expect(doConversion).not.toThrow();
    });
  });

  describe("eip712Json", () => {
    it("should generate EIP712 JSON for TransactionSigningPayload", () => {
      const call = {
        bank: {
          transfer: {
            to: {
              Standard:
                "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
            },
            coins: {
              amount: "1000",
              token_id:
                "token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6",
            },
          },
        },
      };

      const transactionSigningPayload = {
        V0: {
          runtime_call: call,
          uniqueness: { generation: "0" },
          details: {
            max_priority_fee_bips: "1000",
            max_fee: "10000",
            gas_limit: null,
            chain_id: "1",
          },
          address_override: null,
          chain_hash: Array.from(schema.chainHash),
        },
      };

      const txBorsh = schema.jsonToBorsh(
        schema.knownTypeIndex(KnownTypeId.TransactionSigningPayload),
        JSON.stringify(transactionSigningPayload),
      );

      const eip712Json = schema.eip712Json(
        schema.knownTypeIndex(KnownTypeId.TransactionSigningPayload),
        txBorsh,
      );

      // Verify it's valid JSON
      const parsed = JSON.parse(eip712Json);

      // Basic structure checks
      expect(parsed).toHaveProperty("domain");
      expect(parsed).toHaveProperty("message");
      expect(parsed).toHaveProperty("primaryType");
      expect(parsed).toHaveProperty("types");

      expect(parsed.primaryType).toBe("TransactionSigningPayload");
      expect(JSON.stringify(parsed)).toEqual(
        `{"domain":{"name":"TestChain","chainId":"0x10e1","salt":"0x10c4d33e43ee94d78fba9ed555bcc65b75c03fefe27b3ba862a92ea3299d09fc"},"types":{"Bank":[{"type":"Transfer","name":"Transfer"}],"Coins":[{"type":"uint128","name":"amount"},{"type":"string","name":"token_id"}],"EIP712Domain":[{"type":"string","name":"name"},{"type":"uint256","name":"chainId"},{"type":"bytes32","name":"salt"}],"MultiAddressEvmSolana":[{"type":"string","name":"Standard"}],"RuntimeCall":[{"type":"Bank","name":"Bank"}],"TransactionSigningPayload":[{"type":"V0","name":"V0"}],"Transfer":[{"type":"MultiAddressEvmSolana","name":"to"},{"type":"Coins","name":"coins"}],"TxDetails":[{"type":"uint64","name":"max_priority_fee_bips"},{"type":"uint128","name":"max_fee"},{"type":"uint64","name":"chain_id"}],"UniquenessData":[{"type":"uint64","name":"Generation"}],"V0":[{"type":"RuntimeCall","name":"runtime_call"},{"type":"UniquenessData","name":"uniqueness"},{"type":"TxDetails","name":"details"}]},"primaryType":"TransactionSigningPayload","message":{"V0":{"details":{"chain_id":"1","max_fee":"10000","max_priority_fee_bips":"1000"},"runtime_call":{"Bank":{"Transfer":{"coins":{"amount":"1000","token_id":"token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6"},"to":{"Standard":"sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"}}}},"uniqueness":{"Generation":"0"}}}}`,
      );
    });
  });

  describe("eip712SigningHash", () => {
    it("should generate EIP712 signing hash for TransactionSigningPayload", () => {
      const call = {
        bank: {
          transfer: {
            to: {
              Standard:
                "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
            },
            coins: {
              amount: "1000",
              token_id:
                "token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6",
            },
          },
        },
      };

      const transactionSigningPayload = {
        V0: {
          runtime_call: call,
          uniqueness: { generation: "0" },
          details: {
            max_priority_fee_bips: "1000",
            max_fee: "10000",
            gas_limit: null,
            chain_id: "1",
          },
          address_override: null,
          chain_hash: Array.from(schema.chainHash),
        },
      };

      const txBorsh = schema.jsonToBorsh(
        schema.knownTypeIndex(KnownTypeId.TransactionSigningPayload),
        JSON.stringify(transactionSigningPayload),
      );

      const signingHash = schema.eip712SigningHash(
        schema.knownTypeIndex(KnownTypeId.TransactionSigningPayload),
        txBorsh,
      );

      // Should return a 32-byte hash
      expect(signingHash).toHaveLength(32);
      expect(bytesToHex(signingHash)).toEqual(
        "f0ee17fcee4284e8c40892c5b837870b5e89d3ed7197ddbe51bf42edf38af30b",
      );
    });
  });
});
