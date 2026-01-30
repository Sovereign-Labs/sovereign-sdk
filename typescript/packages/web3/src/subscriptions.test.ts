import IsoWebSocket from "isomorphic-ws";
import { WebSocket } from "partysocket";
import { afterEach, describe, expect, it, test, vi } from "vitest";
import {
  _subscriptions,
  createSubscription,
  httpToWebSocket,
  subscribe,
} from "./subscriptions";

vi.mock("partysocket", () => {
  const WebSocket = vi.fn();
  WebSocket.prototype.addEventListener = vi.fn();
  WebSocket.prototype.close = vi.fn();
  return { WebSocket };
});

vi.mock("isomorphic-ws", () => {
  const IsoWebSocket = vi.fn();
  return { default: IsoWebSocket };
});

test("httpToWebSocket", () => {
  expect(httpToWebSocket("http://localhost:12346/sequencer")).toBe(
    "ws://localhost:12346/sequencer",
  );
  expect(httpToWebSocket("https://localhost:12346/ledger")).toBe(
    "wss://localhost:12346/ledger",
  );
});

describe("subscriptions", () => {
  describe("subscribe", () => {
    afterEach(() => {
      vi.clearAllMocks();
      for (const key of Object.keys(_subscriptions)) {
        delete _subscriptions[key];
      }
    });

    const TEST_URL = "ws://test.com";

    it("should create new WebSocket connection for first subscription", () => {
      const callback = vi.fn();
      subscribe(TEST_URL, callback);

      expect(WebSocket).toHaveBeenCalledWith(TEST_URL, [], {
        WebSocket: IsoWebSocket,
      });
      expect(WebSocket.prototype.addEventListener).toHaveBeenCalledWith(
        "message",
        expect.any(Function),
      );
    });
    it("should reuse existing WebSocket for multiple subscriptions", () => {
      const callback1 = vi.fn();
      const callback2 = vi.fn();

      subscribe(TEST_URL, callback1);
      subscribe(TEST_URL, callback2);

      expect(WebSocket).toHaveBeenCalledTimes(1);
    });

    it("should call all callbacks when message is received", () => {
      const callback1 = vi.fn();
      const callback2 = vi.fn();
      const testData = { foo: "bar" };

      subscribe(TEST_URL, callback1);
      subscribe(TEST_URL, callback2);

      // Get the message handler that was registered
      const messageHandler = (WebSocket.prototype.addEventListener as any).mock
        .calls[0][1];
      messageHandler({ data: JSON.stringify(testData) });

      expect(callback1).toHaveBeenCalledWith(testData);
      expect(callback2).toHaveBeenCalledWith(testData);
    });
    describe("unsubscribe", () => {
      it("should remove callback but keep connection with remaining subscribers", () => {
        const callback1 = vi.fn();
        const callback2 = vi.fn();
        const testData = { foo: "bar" };

        const subscription1 = subscribe(TEST_URL, callback1);
        subscribe(TEST_URL, callback2);

        const messageHandler = (WebSocket.prototype.addEventListener as any)
          .mock.calls[0][1];

        subscription1.unsubscribe();

        messageHandler({ data: JSON.stringify(testData) });

        expect(callback1).not.toHaveBeenCalled();
        expect(callback2).toHaveBeenCalledWith(testData);
        expect(WebSocket.prototype.close).not.toHaveBeenCalled();
      });
      it("should close WebSocket when last subscriber unsubscribes", () => {
        const callback = vi.fn();
        const subscription = subscribe(TEST_URL, callback);

        subscription.unsubscribe();

        expect(WebSocket.prototype.close).toHaveBeenCalled();
      });
      it("should handle unsubscribe for non-existent subscription", () => {
        const callback = vi.fn();
        const subscription = subscribe(TEST_URL, callback);

        // Unsubscribe twice should not throw
        subscription.unsubscribe();
        subscription.unsubscribe();

        expect(WebSocket.prototype.close).toHaveBeenCalledTimes(1);
      });
    });
    it("should expose subscription URL", () => {
      const callback = vi.fn();
      const subscription = subscribe(TEST_URL, callback);

      expect(subscription.url).toBe(TEST_URL);
    });
  });
});

describe("createSubscription", () => {
  it("should throw error for invalid subscription type", () => {
    const callback = async () => {};
    // @ts-expect-error - Testing invalid subscription type
    expect(() => createSubscription("invalid", callback, {} as any)).toThrow(
      "Invalid subscription type 'invalid'",
    );
  });
});
