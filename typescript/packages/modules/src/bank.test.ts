import { SovereignClient } from "@sovereign-sdk/web3";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Bank } from "./bank";

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

  describe("totalSupply", () => {
    const mockTokenId = "token_123";
    const mockGasTokenId = "gas_token_456";

    beforeEach(() => {
      // Mock gasTokenId method
      vi.spyOn(bank, "gasTokenId").mockResolvedValue(mockGasTokenId);
    });

    it("should return total supply for a specific token", async () => {
      const mockResponse = {
        amount: "1000000000000000000000000",
        token_id: mockTokenId,
      };

      mockClient.get.mockResolvedValue(mockResponse);

      const result = await bank.totalSupply(mockTokenId);

      expect(mockClient.get).toHaveBeenCalledWith(
        `/modules/bank/tokens/${mockTokenId}/total-supply`,
      );
      expect(result).toBe(BigInt("1000000000000000000000000"));
    });

    it("should return total supply for gas token when no tokenId provided", async () => {
      const mockResponse = {
        amount: "500000000000000000000000",
        token_id: mockGasTokenId,
      };

      mockClient.get.mockResolvedValue(mockResponse);

      const result = await bank.totalSupply();

      expect(mockClient.get).toHaveBeenCalledWith(
        `/modules/bank/tokens/${mockGasTokenId}/total-supply`,
      );
      expect(result).toBe(BigInt("500000000000000000000000"));
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

      await expect(bank.totalSupply(mockTokenId)).rejects.toEqual(apiError);
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
