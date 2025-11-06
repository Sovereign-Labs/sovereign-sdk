import { Ed25519Signer } from "@sovereign-sdk/signers";
import { type Rollup, createStandardRollup } from "@sovereign-sdk/web3";
import { beforeAll, describe, expect, it } from "vitest";
import { Bank } from "../src";

const privateKey = new Uint8Array([
  117, 251, 248, 217, 135, 70, 194, 105, 46, 80, 41, 66, 185, 56, 200, 35, 121,
  253, 9, 234, 159, 91, 96, 212, 211, 158, 135, 225, 180, 36, 104, 253,
]);
const signer = new Ed25519Signer(privateKey);
const NON_EXISTENT_ADDRESS =
  "sov1z3lak4ph8m367vqhwu3x4dzyprhd5ex60frs5js0p2ptjmjere6";
const VALID_GAS_ADDRESS =
  "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf";

describe("bank", async () => {
  let rollup: Rollup<any, any>;
  const createdTokenInitBalance = "20000";
  let createdTokenId: string;

  beforeAll(async () => {
    rollup = await createStandardRollup();

    const { response } = await rollup.call(
      {
        bank: {
          create_token: {
            token_name: "BankIntegrationTest3",
            initial_balance: createdTokenInitBalance,
            token_decimals: 8,
            supply_cap: "100000055555",
            mint_to_address: {
              Standard: VALID_GAS_ADDRESS,
            },
            admins: [],
          },
        },
      },
      { signer },
    );
    createdTokenId = (response.events![0].value.token_created as any)!.coins!
      .token_id;
  });

  describe("balance()", async () => {
    it("should return 0 for an account with no balance", async () => {
      const bank = new Bank(rollup);
      const actual = await bank.balance(NON_EXISTENT_ADDRESS);
      expect(actual).toBe(BigInt(0));
    });

    it("should return the balance for an existing account", async () => {
      const bank = new Bank(rollup);
      const actual = await bank.balance(VALID_GAS_ADDRESS);
      // Other integration tests might modify the balance, lets just make sure it returns _something_
      expect(actual).toBeGreaterThan(BigInt(0));
    });

    it("should use the tokenId parameter if it is supplied", async () => {
      const bank = new Bank(rollup);
      const actual = await bank.balance(VALID_GAS_ADDRESS, createdTokenId);
      expect(actual).toBe(BigInt(createdTokenInitBalance));
    });
  });

  describe("gasTokenId()", async () => {
    it("should return the expected gas token id", async () => {
      const bank = new Bank(rollup);
      const actual = await bank.gasTokenId();
      expect(actual).toBe(
        "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7",
      );
    });
  });

  describe("totalSupply()", async () => {
    it("should return the expected total supply", async () => {
      const bank = new Bank(rollup);
      const actual = await bank.totalSupply();
      expect(actual).toBe(BigInt("10030000000000000"));
    });
    it("should use the tokenId parameter if it is supplied", async () => {
      const bank = new Bank(rollup);
      const actual = await bank.totalSupply(createdTokenId);
      expect(actual).toBe(BigInt(createdTokenInitBalance));
    });
  });
});
