type ConnectButtonProps = {
  isConnected: boolean;
  walletAddress: string | null;
  isLoading: boolean;
  isPhantomAvailable: boolean;
  onConnect: () => Promise<void>;
  onDisconnect: () => Promise<void>;
};

function shorten(address: string | null): string {
  if (!address) {
    return "";
  }

  if (address.length <= 12) {
    return address;
  }

  return `${address.slice(0, 6)}...${address.slice(-4)}`;
}

export default function ConnectButton({
  isConnected,
  walletAddress,
  isLoading,
  isPhantomAvailable,
  onConnect,
  onDisconnect,
}: ConnectButtonProps) {
  const handleClick = () => {
    if (!isPhantomAvailable || isLoading) {
      return;
    }

    if (isConnected) {
      void onDisconnect();
      return;
    }

    void onConnect();
  };

  const label = !isPhantomAvailable
    ? "Phantom Not Installed"
    : isConnected
      ? `Disconnect ${shorten(walletAddress)}`
      : "Connect Phantom";

  return (
    <button onClick={handleClick} disabled={!isPhantomAvailable || isLoading}>
      {isLoading ? "Working..." : label}
    </button>
  );
}
