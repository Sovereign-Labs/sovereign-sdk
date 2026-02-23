import { useEffect, useState } from "react";
import { createSolanaSignableRollup, SovereignClient } from "@sovereign-sdk/web3";
import ConnectButton from "./ConnectButton.tsx";
import "./App.css";
import {
  type PhantomProvider,
  PhantomSigner,
  getPhantomProvider,
} from "./phantom";
import type { RuntimeCall } from "./types";

type TxResponse = SovereignClient.SovereignSDK.Sequencer.TxCreateResponse;

const ROLLUP_URL = import.meta.env.VITE_ROLLUP_URL || "http://localhost:12346";
const SOLANA_ENDPOINT =
  import.meta.env.VITE_SOLANA_ENDPOINT || "/sequencer/accept-solana-offchain-tx";
const CHAIN_ID_FROM_ENV = Number(import.meta.env.VITE_CHAIN_ID || "4321");

const DEFAULT_TX = {
  bank: {
    create_token: {
      token_name: "My Token",
      token_decimals: 8,
      initial_balance: "1000000000",
      mint_to_address: "<wallet_address>",
      admins: [],
      supply_cap: "100000000000",
    },
  },
};

export default function App() {
  const [walletProvider, setWalletProvider] = useState<PhantomProvider | null>(
    null,
  );
  const [walletAddress, setWalletAddress] = useState<string | null>(null);
  const [isWalletReady, setIsWalletReady] = useState(false);
  const [isConnecting, setIsConnecting] = useState(false);
  const [txResult, setTxResult] = useState<TxResponse | null>(null);
  const [txError, setTxError] = useState<string>("");
  const [isLoading, setIsLoading] = useState(false);
  const [isSuccess, setIsSuccess] = useState(false);
  const [txInput, setTxInput] = useState(JSON.stringify(DEFAULT_TX, null, 2));

  useEffect(() => {
    const provider = getPhantomProvider();
    setWalletProvider(provider);

    if (!provider) {
      setIsWalletReady(true);
      return;
    }

    let cancelled = false;

    const hydrateSession = async () => {
      try {
        const response = await provider.connect({ onlyIfTrusted: true });
        if (!cancelled) {
          setWalletAddress(response.publicKey.toString());
        }
      } catch {
        if (!cancelled) {
          setWalletAddress(null);
        }
      } finally {
        if (!cancelled) {
          setIsWalletReady(true);
        }
      }
    };

    void hydrateSession();

    return () => {
      cancelled = true;
    };
  }, []);

  const handleConnect = async () => {
    if (!walletProvider) {
      setTxError("Phantom wallet extension not detected.");
      return;
    }

    setIsConnecting(true);
    setTxError("");

    try {
      const response = await walletProvider.connect();
      setWalletAddress(response.publicKey.toString());
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : "Failed to connect wallet";
      setTxError(message);
    } finally {
      setIsConnecting(false);
    }
  };

  const handleDisconnect = async () => {
    if (!walletProvider) {
      return;
    }

    setIsConnecting(true);
    setTxError("");

    try {
      await walletProvider.disconnect();
      setWalletAddress(null);
      setTxResult(null);
      setIsSuccess(false);
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Failed to disconnect wallet";
      setTxError(message);
    } finally {
      setIsConnecting(false);
    }
  };

  const handleSignAndSend = async () => {
    if (!walletProvider || !walletAddress) {
      setTxError("Phantom wallet is not connected.");
      return;
    }

    setIsLoading(true);
    setTxError("");
    setTxResult(null);
    setIsSuccess(false);

    try {
      // Parse and prepare transaction
      const txString = txInput.replace("<wallet_address>", walletAddress);
      const parsedTx: RuntimeCall = JSON.parse(txString);

      // Create rollup client and Phantom signer
      const rollupConfig = Number.isInteger(CHAIN_ID_FROM_ENV)
        ? {
            url: ROLLUP_URL,
            context: {
              defaultTxDetails: {
                chain_id: CHAIN_ID_FROM_ENV,
              },
            },
          }
        : { url: ROLLUP_URL };
      const rollup = await createSolanaSignableRollup<RuntimeCall>(
        rollupConfig,
        SOLANA_ENDPOINT,
      );
      const signer = new PhantomSigner(walletProvider);

      // Sign and send as a Solana offchain message
      const result = await rollup.call(parsedTx, {
        signer,
        authenticator: "solanaSimple",
      });
      setTxResult(result.response ?? null);
      setIsSuccess(true);
    } catch (err: unknown) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const e = err as any;
      const details = e.error?.details || e.details;
      setTxError(e.message + (details ? "\n\n" + JSON.stringify(details, null, 2) : ""));
    } finally {
      setIsLoading(false);
    }
  };

  if (!isWalletReady) {
    return <div className="container">Loading Phantom...</div>;
  }

  return (
    <div className="container">
      <header>
        <h1>Phantom Solana Offchain Example</h1>
        <ConnectButton
          isConnected={Boolean(walletAddress)}
          walletAddress={walletAddress}
          isLoading={isConnecting}
          isPhantomAvailable={Boolean(walletProvider)}
          onConnect={handleConnect}
          onDisconnect={handleDisconnect}
        />
      </header>

      {!walletProvider ? (
        <section className="connect-prompt">
          <p>
            Phantom was not detected. Install the Phantom browser extension and
            refresh this page.
          </p>
        </section>
      ) : walletAddress ? (
        <section className="transaction-section">
          <h3>Send Solana Offchain Transaction</h3>

          <label htmlFor="tx-input">Transaction Data (JSON):</label>
          <textarea
            id="tx-input"
            value={txInput}
            onChange={(e) => setTxInput(e.target.value)}
            placeholder="Enter transaction JSON..."
          />

          <button
            onClick={handleSignAndSend}
            disabled={isLoading}
            className="primary-button"
          >
            {isLoading ? "Processing..." : "Sign with Phantom and Send"}
          </button>

          {txError && (
            <div className="message error">
              <strong>Error:</strong>
              <pre>{txError}</pre>
            </div>
          )}

          {isSuccess && (
            <div className="message success">
              <strong>Transaction Submitted Successfully!</strong>
              {txResult ? (
                <>
                  <div>
                    <strong>Hash:</strong>{" "}
                    <code>{txResult.id || "N/A"}</code>
                  </div>
                  <div>
                    <strong>Status:</strong> {txResult.status || "N/A"}
                  </div>
                  {txResult.events && txResult.events.length > 0 && (
                    <div>
                      <strong>Events:</strong>
                      <pre>{JSON.stringify(txResult.events, null, 2)}</pre>
                    </div>
                  )}
                </>
              ) : (
                <div>Transaction was sent to the rollup.</div>
              )}
            </div>
          )}
        </section>
      ) : (
        <section className="connect-prompt">
          <p>Connect Phantom to sign and send Solana offchain messages</p>
        </section>
      )}
    </div>
  );
}
