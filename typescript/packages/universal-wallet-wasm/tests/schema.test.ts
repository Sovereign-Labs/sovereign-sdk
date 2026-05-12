import { describe, it, expect } from "vitest";
import { Schema, KnownTypeId } from "@sovereign-sdk/universal-wallet-wasm";
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
        "0747c78c9a62a856f5208d9a980136996209d96bfacc6aa9db8c5da2b8322792";
      const actual = bytesToHex(schema.chainHash);

      expect(actual).toEqual(expected);
    });
  });
  describe("metadataHash", () => {
    it("should restore the metadata hash successfully", () => {
      const expected =
        "1b7964c74362ea00edd356995713768b068d69a0d13da3bf143959307a228e06";
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
          JSON.stringify(call)
        )
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
          JSON.stringify(call)
        );
      expect(doConversion).toThrow(
        "Expected type or field __SovVirtualWallet_CallMessage_CreateToken.token_name, but it was not present"
      );
    });
    it("should allow strings to serialize as u128", () => {
      const addr = hexToBytes(
        "b7e23f9dc86a1547ee09d82a5c8f3610d975e2c84fb61038a719e524"
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
          JSON.stringify(call)
        );
      expect(doConversion).not.toThrow();
    });
  });

  describe("eip712Json", () => {
    it("should generate EIP712 JSON for UnsignedTransaction", () => {
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

      const unsignedTransaction = {
        V0: {
          runtime_call: call,
          uniqueness: { generation: "0" },
          details: {
            max_priority_fee_bips: "1000",
            max_fee: "10000",
            gas_limit: null,
            chain_id: "1",
          },
        },
      };

      const txBorsh = schema.jsonToBorsh(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        JSON.stringify(unsignedTransaction)
      );

      const eip712Json = schema.eip712Json(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        txBorsh
      );

      // Verify it's valid JSON
      const parsed = JSON.parse(eip712Json);

      // Basic structure checks
      expect(parsed).toHaveProperty("domain");
      expect(parsed).toHaveProperty("message");
      expect(parsed).toHaveProperty("primaryType");
      expect(parsed).toHaveProperty("types");

      expect(parsed.primaryType).toBe("UnsignedTransaction");
      expect(JSON.stringify(parsed)).toEqual(
        `{"domain":{"name":"TestChain","chainId":"0x10e1","salt":"0x0747c78c9a62a856f5208d9a980136996209d96bfacc6aa9db8c5da2b8322792"},"types":{"Bank":[{"type":"Transfer","name":"Transfer"}],"Coins":[{"type":"uint128","name":"amount"},{"type":"string","name":"token_id"}],"EIP712Domain":[{"type":"string","name":"name"},{"type":"uint256","name":"chainId"},{"type":"bytes32","name":"salt"}],"MultiAddressEvmSolana":[{"type":"string","name":"Standard"}],"RuntimeCall":[{"type":"Bank","name":"Bank"}],"Transfer":[{"type":"MultiAddressEvmSolana","name":"to"},{"type":"Coins","name":"coins"}],"TxDetails":[{"type":"uint64","name":"max_priority_fee_bips"},{"type":"uint128","name":"max_fee"},{"type":"uint64","name":"chain_id"}],"UniquenessData":[{"type":"uint64","name":"Generation"}],"UnsignedTransaction":[{"type":"V0","name":"V0"}],"V0":[{"type":"RuntimeCall","name":"runtime_call"},{"type":"UniquenessData","name":"uniqueness"},{"type":"TxDetails","name":"details"}]},"primaryType":"UnsignedTransaction","message":{"V0":{"details":{"chain_id":"1","max_fee":"10000","max_priority_fee_bips":"1000"},"runtime_call":{"Bank":{"Transfer":{"coins":{"amount":"1000","token_id":"token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6"},"to":{"Standard":"sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"}}}},"uniqueness":{"Generation":"0"}}}}`
      );
    });
  });

  describe("eip712SigningHash", () => {
    it("should generate EIP712 signing hash for UnsignedTransaction", () => {
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

      const unsignedTransaction = {
        V0: {
          runtime_call: call,
          uniqueness: { generation: "0" },
          details: {
            max_priority_fee_bips: "1000",
            max_fee: "10000",
            gas_limit: null,
            chain_id: "1",
          },
        },
      };

      const txBorsh = schema.jsonToBorsh(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        JSON.stringify(unsignedTransaction)
      );

      const signingHash = schema.eip712SigningHash(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        txBorsh
      );

      // Should return a 32-byte hash
      expect(signingHash).toHaveLength(32);
      expect(bytesToHex(signingHash)).toEqual(
        "e21ca89ff493401ac0371c8bc7dc5782c585c0c9de8a3583a4753cb203f85999"
      );
    });
  });
});
