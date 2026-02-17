import { SovereignClient } from "@sovereign-sdk/web3";
import { bech32m } from "bech32";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Bank, getTokenId } from "./bank";

describe("getTokenId", () => {
  it("should match the Rust get_token_id output", () => {
    // sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv decodes to [11; 28]
    const originator = new Uint8Array(28).fill(11);
    const tokenName = "test-token";
    const decimals = 6;

    const tokenId = getTokenId(originator, tokenName, decimals);

    // Expected token ID from Rust: token_1em6nucpnzadj2zyvdg6yk5rt754kdy07d4qv764mc44hfp30a5rq0539v4
    const expected = bech32m.decode(
      "token_1em6nucpnzadj2zyvdg6yk5rt754kdy07d4qv764mc44hfp30a5rq0539v4",
    );
    const expectedBytes = new Uint8Array(bech32m.fromWords(expected.words));

    expect(tokenId).toEqual(expectedBytes);
  });
});

describe("Bank", () => {
  let mockRollup: any;
  let mockClient: any;
  let bank: Bank;

  beforeEach(() => {
    vi.clearAllMocks();

    mockClient = {
      get: vi.fn(),
    };

    mockRollup = {
      http: mockClient,
    };

    bank = new Bank(mockRollup);
  });

  describe("balance", () => {
    const mockAddress = "0x1234567890abcdef";
    const mockTokenId = "token_123";
    const mockGasTokenId = "gas_token_456";

    beforeEach(() => {
      // Mock gasTokenId method
      vi.spyOn(bank, "gasTokenId").mockResolvedValue(mockGasTokenId);
    });

    it("should return balance for a specific token", async () => {
      const mockResponse = {
        amount: "1000000000000000000",
        token_id: mockTokenId,
      };

      mockClient.get.mockResolvedValue(mockResponse);

      const result = await bank.balance(mockAddress, mockTokenId);

      expect(mockClient.get).toHaveBeenCalledWith(
        `/modules/bank/tokens/${mockTokenId}/balances/${mockAddress}`,
      );
      expect(result).toBe(BigInt("1000000000000000000"));
    });

    it("should return balance for gas token when no tokenId provided", async () => {
      const mockResponse = {
        amount: "500000000000000000",
        token_id: mockGasTokenId,
      };

      mockClient.get.mockResolvedValue(mockResponse);

      const result = await bank.balance(mockAddress);

      expect(mockClient.get).toHaveBeenCalledWith(
        `/modules/bank/tokens/${mockGasTokenId}/balances/${mockAddress}`,
      );
      expect(result).toBe(BigInt("500000000000000000"));
    });

    it("should return 0 for missing account", async () => {
      const apiError = new SovereignClient.APIError(
        404,
        {
          message: "Balance 'sov1lkjo2jiojoj' not found",
        },
        undefined,
        undefined,
      );

      mockClient.get.mockRejectedValue(apiError);

      const result = await bank.balance(mockAddress, mockTokenId);

      expect(result).toBe(BigInt(0));
    });

    it("should throw error for non-404 API errors", async () => {
      const apiError = {
        status: 500,
        error: {
          message: "Internal server error",
        },
      };

      mockClient.get.mockRejectedValue(apiError);

      await expect(bank.balance(mockAddress, mockTokenId)).rejects.toEqual(
        apiError,
      );
    });

    it("should throw error for non-API errors", async () => {
      const networkError = new Error("Network error");
      mockClient.get.mockRejectedValue(networkError);

      await expect(bank.balance(mockAddress, mockTokenId)).rejects.toThrow(
        "Network error",
      );
    });

    it("should throw error for 404 with non-balance error title", async () => {
      const apiError = {
        status: 404,
        error: {
          errors: [
            {
              title: "Something else not found",
            },
          ],
        },
      };

      mockClient.get.mockRejectedValue(apiError);

      await expect(bank.balance(mockAddress, mockTokenId)).rejects.toEqual(
        apiError,
      );
    });

    it("should throw error for 404 with empty errors array", async () => {
      const apiError = {
        status: 404,
        error: {
          errors: [],
        },
      };

      mockClient.get.mockRejectedValue(apiError);

      await expect(bank.balance(mockAddress, mockTokenId)).rejects.toEqual(
        apiError,
      );
    });
  });

  describe("tokenMetadata", () => {
    // Valid bech32m token ID with decimals = 89 (last byte)
    const mockTokenId =
      "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7";

    beforeEach(() => {
      // Mock gasTokenId method
      vi.spyOn(bank, "gasTokenId").mockResolvedValue(mockTokenId);
    });

    it("should return token metadata for a specific token", async () => {
      const mockResponse = {
        key: mockTokenId,
        value: {
          name: "Test Token",
          total_supply: "1000000000000000000000000",
          supply_cap: "2000000000000000000000000",
          admins: [{ user: "sov1abc123" }, { module: "sov1mod456" }],
        },
      };

      mockClient.get.mockResolvedValue(mockResponse);

      const result = await bank.tokenMetadata(mockTokenId);

      expect(mockClient.get).toHaveBeenCalledWith(
        `/modules/bank/state/tokens/items/${mockTokenId}`,
      );
      expect(result).toEqual({
        name: "Test Token",
        decimals: 89,
        totalSupply: BigInt("1000000000000000000000000"),
        supplyCap: BigInt("2000000000000000000000000"),
        admins: ["sov1abc123", "sov1mod456"],
      });
    });

    it("should return token metadata for gas token when no tokenId provided", async () => {
      const mockResponse = {
        key: mockTokenId,
        value: {
          name: "Gas Token",
          total_supply: "500000000000000000000000",
          supply_cap: "1000000000000000000000000",
          admins: [{ derived: "sov1derived789" }],
        },
      };

      mockClient.get.mockResolvedValue(mockResponse);

      const result = await bank.tokenMetadata();

      expect(mockClient.get).toHaveBeenCalledWith(
        `/modules/bank/state/tokens/items/${mockTokenId}`,
      );
      expect(result).toEqual({
        name: "Gas Token",
        decimals: 89,
        totalSupply: BigInt("500000000000000000000000"),
        supplyCap: BigInt("1000000000000000000000000"),
        admins: ["sov1derived789"],
      });
    });

    it("should throw error when API request fails", async () => {
      const apiError = {
        status: 500,
        error: {
          errors: [
            {
              title: "Internal server error",
            },
          ],
        },
      };

      mockClient.get.mockRejectedValue(apiError);

      await expect(bank.tokenMetadata(mockTokenId)).rejects.toEqual(apiError);
    });
  });

  describe("gasTokenId", () => {
    it("should return cached gas token ID on subsequent calls", async () => {
      const mockResponse = {
        token_id: "gas_token_123",
      };

      mockClient.get.mockResolvedValue(mockResponse);

      // First call
      const result1 = await bank.gasTokenId();
      expect(result1).toBe("gas_token_123");
      expect(mockClient.get).toHaveBeenCalledTimes(1);

      // Second call should use cache
      const result2 = await bank.gasTokenId();
      expect(result2).toBe("gas_token_123");
      expect(mockClient.get).toHaveBeenCalledTimes(1); // Still only called once
    });

    it("should make API call to get gas token ID on first call", async () => {
      const mockResponse = {
        token_id: "gas_token_456",
      };

      mockClient.get.mockResolvedValue(mockResponse);

      const result = await bank.gasTokenId();

      expect(mockClient.get).toHaveBeenCalledWith(
        "/modules/bank/tokens/gas_token",
      );
      expect(result).toBe("gas_token_456");
    });

    it("should throw error when API request fails", async () => {
      const apiError = {
        status: 500,
        error: {
          message: "Internal server error",
        },
      };

      mockClient.get.mockRejectedValue(apiError);

      await expect(bank.gasTokenId()).rejects.toEqual(apiError);
    });
  });
});
