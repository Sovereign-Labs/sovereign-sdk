import type { InterfaceAbi } from "ethers";
import type { PrivateKeyAccount } from "viem/accounts";
import type { PublicClient } from "viem";
import type { JsonRpcProvider, Wallet } from "ethers";

export type Outcome = "PASS" | "FAIL" | "NOT_SUPPORTED";
export type ClientLibrary = "ethers" | "viem" | "raw";

export interface RpcErrorShape {
  code: number | null;
  message: string;
  data?: unknown;
  raw?: unknown;
}

export interface EndpointObservation {
  normalized?: unknown;
  raw?: unknown;
  error?: RpcErrorShape;
  unsupported?: boolean;
  notes?: string[];
}

export interface CheckResult {
  name: string;
  library: ClientLibrary;
  rpcMethods: string[];
  requests: {
    anvil: unknown[];
    rollup: unknown[];
  };
  anvil: EndpointObservation;
  rollup: EndpointObservation;
  outcome: Outcome;
  diff?: unknown;
  notes?: string[];
}

export interface ReportSummary {
  total: number;
  pass: number;
  fail: number;
  notSupported: number;
}

export interface CompareReport {
  generatedAt: string;
  anvil: {
    rpcUrl: string;
    chainId: string;
    clientVersion?: string;
  };
  rollup: {
    rpcUrl: string;
    chainId: string;
    clientVersion?: string;
  };
  summary: ReportSummary;
  checks: CheckResult[];
}

export interface ContractArtifact {
  abi: InterfaceAbi;
  bytecode: string;
}

export interface CompiledContracts {
  kitchenSink: ContractArtifact;
  delegateTarget: ContractArtifact;
  callReceiver: ContractArtifact;
  create2Child: ContractArtifact;
}

export interface DeploymentState {
  kitchenSink: string;
  delegateTarget: string;
  callReceiver: string;
  deploymentReceiptContractAddressPresent: boolean;
}

export interface EndpointConfig {
  name: "anvil" | "rollup";
  rpcUrl: string;
  privateKey: `0x${string}`;
  chainId: bigint;
}

export interface EndpointRuntime {
  name: "anvil" | "rollup";
  rpcUrl: string;
  chainId: bigint;
  provider: JsonRpcProvider;
  wallet: Wallet;
  viemClient: PublicClient;
  account: PrivateKeyAccount;
  deployment: DeploymentState;
}

export interface HarnessContext {
  contracts: CompiledContracts;
  anvil: EndpointRuntime;
  rollup: EndpointRuntime;
}

export interface EndpointExecution {
  observation: EndpointObservation;
  requests: unknown[];
}
