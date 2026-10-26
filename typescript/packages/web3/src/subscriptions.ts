import type SovereignSDK from "@sovereign-sdk/client";
import IsoWebSocket from "isomorphic-ws";
import { WebSocket } from "partysocket";
import { SovereignError } from "./errors";

type Url = string;

export function httpToWebSocket(url: string) {
  return url.replace(/^http(s?):\/\//, (_, s) => `ws${s}://`);
}

export type Subscription = {
  /** Url providing the subscription. */
  url: Url;
  /** Unsubscribe this subscription. */
  unsubscribe: () => void;
};

type SubscriptionEntry = {
  websocket: WebSocket;
  // biome-ignore lint/suspicious/noExplicitAny: allow
  callbacks: Array<(...args: any[]) => void>;
};

export const _subscriptions: Record<Url, SubscriptionEntry> = {};

/** Subscribe to the provided websocket url & call the provided callback on messages received. */
export function subscribe(
  url: Url,
  // biome-ignore lint/suspicious/noExplicitAny: allow
  callback: (...args: any[]) => void,
): Subscription {
  const subscription = _subscriptions[url];

  if (!subscription) {
    const websocket = new WebSocket(url, [], { WebSocket: IsoWebSocket });
    // bun compatibility: https://github.com/partykit/partykit/issues/774
    websocket.binaryType = "arraybuffer";

    _subscriptions[url] = {
      websocket,
      callbacks: [callback],
    };

    websocket.addEventListener("message", (message) => {
      const data = JSON.parse(message.data);
      const callbacks = _subscriptions[url]?.callbacks;
      if (callbacks) {
        for (const cb of callbacks) {
          cb(data);
        }
      }
    });
    websocket.addEventListener("error", (e) =>
      console.log("Websocket error", e),
    );
  } else {
    subscription.callbacks.push(callback);
  }

  const callbackIndex = _subscriptions[url]?.callbacks.length - 1;

  return {
    get url() {
      return url;
    },
    unsubscribe() {
      const subscription = _subscriptions[url];

      if (!subscription) {
        return;
      }

      subscription.callbacks.splice(callbackIndex, 1);

      if (!subscription.callbacks.length) {
        subscription.websocket.close();
        delete _subscriptions[url];
      }
    },
  };
}

export type EventPayload = {
  tx_hash: string;
  type: "event";
  number: number;
  key: string;
  value: Record<string, unknown>;
  module: {
    type: "moduleRef";
    name: string;
  };
  /**
   * The run of numbering this copy of the event is served under.
   *
   * Present on every event served by the sequencer, from its cache or read back from the
   * ledger: the sequencer's epoch at the time of serving. Before applying this event to a
   * state snapshot, check it against the `x-sov-event-epoch` header that snapshot came with:
   * if they differ, the sequencer has discarded soft confirmations since the snapshot was
   * taken and the client must resynchronise. Comparing `number` alone is not enough, because
   * a rollback that re-emits past where it rewound leaves the number higher than before.
   *
   * Absent on events from the node's ledger API, which has no epoch. An absent epoch counts
   * as the all-zero epoch, which only a publisher that never reissues numbers, such as the
   * standard sequencer, reports. On a DA layer without instant finality, a DA reorg can still
   * renumber events in non-finalized slots; no epoch guards against that.
   */
  epoch?: string;
};

export interface SubscriptionToCallbackMap {
  events: (event: EventPayload) => Promise<void>;
}

export function createSubscription<T extends keyof SubscriptionToCallbackMap>(
  subscription: T,
  callback: SubscriptionToCallbackMap[T],
  client: SovereignSDK,
): Subscription {
  switch (subscription) {
    case "events": {
      const url = client.buildURL("/sequencer/events/ws", {});
      return subscribe(
        httpToWebSocket(url),
        callback as SubscriptionToCallbackMap["events"],
      );
    }
    default:
      throw new SovereignError(`Invalid subscription type '${subscription}'`);
  }
}
