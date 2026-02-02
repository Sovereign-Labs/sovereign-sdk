#!/usr/bin/env node
/**
 * CLI script to add chain hash overrides to constants.toml
 *
 * This script fetches the current chain hash from a running rollup node
 * and adds it as an override entry in constants.toml for upgrade scenarios.
 *
 * Usage:
 *   pnpm add-chain-hash-override -c ../../../constants.toml --blocks-from-now 10000
 *   pnpm add-chain-hash-override -c ../../../constants.toml --end-height 50000
 *   pnpm add-chain-hash-override -c ../../../constants.toml -b 10000 -g 100 --dry-run
 */

import { Command } from "commander";
import * as fs from "node:fs";
import * as path from "node:path";
import { parse as parseToml } from "smol-toml";

interface ChainHashOverride {
  start_height: number;
  end_height: number;
  chain_hash: string;
  grace_period: number;
}

interface SchemaResponse {
  chain_hash: string;
  schema: unknown;
}

interface RollupHeightResponse {
  rollup_height: number;
}

interface ConstantsSection {
  CHAIN_HASH_OVERRIDES?: ChainHashOverride[];
  [key: string]: unknown;
}

interface ConstantsToml {
  constants?: ConstantsSection;
  [key: string]: unknown;
}

async function fetchChainHash(url: string): Promise<string> {
  const response = await fetch(`${url}/rollup/schema`);
  if (!response.ok) {
    throw new Error(
      `Failed to fetch chain hash: ${response.status} ${response.statusText}`
    );
  }
  const data = (await response.json()) as SchemaResponse;
  return data.chain_hash;
}

async function fetchRollupHeight(url: string): Promise<number> {
  const response = await fetch(`${url}/sequencer/rollup-height`);
  if (!response.ok) {
    throw new Error(
      `Failed to fetch rollup height: ${response.status} ${response.statusText}`
    );
  }
  const data = (await response.json()) as RollupHeightResponse;
  return data.rollup_height;
}

function parseConstantsToml(content: string): ConstantsToml {
  return parseToml(content) as unknown as ConstantsToml;
}

function formatOverrideInline(override: ChainHashOverride): string {
  return `{start_height = ${override.start_height}, end_height = ${override.end_height}, chain_hash = "${override.chain_hash}", grace_period = ${override.grace_period}}`;
}

function buildOverridesArrayString(overrides: ChainHashOverride[]): string {
  return `[${overrides.map(formatOverrideInline).join(", ")}]`;
}

function addOverrideToToml(
  content: string,
  newOverride: ChainHashOverride,
  existingOverrides: ChainHashOverride[]
): string {
  const allOverrides = [...existingOverrides, newOverride];
  const newArrayValue = buildOverridesArrayString(allOverrides);
  const overrideLine = `CHAIN_HASH_OVERRIDES = ${newArrayValue}`;

  // If CHAIN_HASH_OVERRIDES already exists, replace it
  const overrideRegex = /^CHAIN_HASH_OVERRIDES\s*=\s*\[.*\]$/m;
  if (overrideRegex.test(content)) {
    return content.replace(overrideRegex, overrideLine);
  }

  // Otherwise, insert after CHAIN_NAME in [constants] section
  const lines = content.split("\n");
  const result: string[] = [];
  let addedOverride = false;
  let inConstantsSection = false;

  for (const line of lines) {
    const trimmedLine = line.trim();

    if (trimmedLine === "[constants]") {
      inConstantsSection = true;
      result.push(line);
      continue;
    }

    // Exiting [constants] section - insert before the new section if not already added
    if (inConstantsSection && trimmedLine.startsWith("[")) {
      if (!addedOverride) {
        result.push(overrideLine);
        addedOverride = true;
      }
      inConstantsSection = false;
    }

    result.push(line);

    // Insert after CHAIN_NAME line
    if (inConstantsSection && !addedOverride && trimmedLine.startsWith("CHAIN_NAME")) {
      result.push(overrideLine);
      addedOverride = true;
    }
  }

  // Fallback: append to end of file if no suitable location found
  if (!addedOverride) {
    result.push("");
    result.push(overrideLine);
  }

  return result.join("\n");
}

async function main(): Promise<void> {
  const program = new Command();

  program
    .name("add-chain-hash-override")
    .description(
      "Add a chain hash override to constants.toml for rollup upgrades"
    )
    .requiredOption(
      "-c, --constants-path <path>",
      "Path to constants.toml file"
    )
    .option("-u, --url <url>", "Rollup node URL", "http://localhost:12346")
    .option(
      "-b, --blocks-from-now <blocks>",
      "Number of blocks from current height until upgrade",
      parseInt
    )
    .option(
      "-e, --end-height <height>",
      "Explicit end height for the override",
      parseInt
    )
    .option(
      "-g, --grace-period <blocks>",
      "Grace period in blocks after end_height. During this time, the both chain hashes will be accepted.",
      parseInt,
      100
    )
    .option("-d, --dry-run", "Preview changes without writing to file", false)
    .parse(process.argv);

  const options = program.opts<{
    constantsPath: string;
    url: string;
    blocksFromNow?: number;
    endHeight?: number;
    gracePeriod: number;
    dryRun: boolean;
  }>();

  // Validate that at least one of blocks-from-now or end-height is provided
  if (options.blocksFromNow === undefined && options.endHeight === undefined) {
    console.error(
      "Error: At least one of --blocks-from-now or --end-height is required"
    );
    process.exit(1);
  }

  // Resolve the constants path
  const constantsPath = path.resolve(options.constantsPath);

  // Check if the file exists
  if (!fs.existsSync(constantsPath)) {
    console.error(`Error: File not found: ${constantsPath}`);
    process.exit(1);
  }

  console.log(`Connecting to rollup at ${options.url}...`);

  // Fetch chain hash and rollup height
  let chainHash: string;
  let rollupHeight: number;

  try {
    [chainHash, rollupHeight] = await Promise.all([
      fetchChainHash(options.url),
      fetchRollupHeight(options.url),
    ]);
  } catch (error) {
    console.error(
      `Error connecting to rollup: ${error instanceof Error ? error.message : error}`
    );
    process.exit(1);
  }

  console.log(`Current chain hash: ${chainHash}`);
  console.log(`Current rollup height: ${rollupHeight}`);
  console.log();

  // Calculate end_height
  let endHeight: number;
  if (options.endHeight !== undefined) {
    endHeight = options.endHeight;
    console.log(`Using explicit end_height: ${endHeight}`);
  } else {
    endHeight = rollupHeight + options.blocksFromNow!;
    console.log(
      `Calculating end_height: ${rollupHeight} + ${options.blocksFromNow} = ${endHeight}`
    );
  }
  console.log();

  // Read and parse the existing constants.toml
  const content = fs.readFileSync(constantsPath, "utf-8");
  const parsed = parseConstantsToml(content);

  // Determine start_height from existing overrides
  const existingOverrides = parsed.constants?.CHAIN_HASH_OVERRIDES ?? [];
  let startHeight: number;

  if (existingOverrides.length === 0) {
    startHeight = 0;
    console.log("No existing overrides found. Using start_height: 0");
  } else {
    const lastOverride = existingOverrides[existingOverrides.length - 1];
    startHeight = lastOverride.end_height;
    console.log(
      `Found ${existingOverrides.length} existing override(s). Using start_height: ${startHeight}`
    );

    // Check if the current chain_hash is already in the overrides
    const existingWithSameHash = existingOverrides.find(
      (o) => o.chain_hash === chainHash
    );
    if (existingWithSameHash) {
      console.error();
      console.error("Error: This chain hash is already present in CHAIN_HASH_OVERRIDES.");
      console.error();
      console.error("Existing override with this hash:");
      console.error(`  start_height: ${existingWithSameHash.start_height}`);
      console.error(`  end_height: ${existingWithSameHash.end_height}`);
      console.error(`  chain_hash: ${existingWithSameHash.chain_hash}`);
      console.error(`  grace_period: ${existingWithSameHash.grace_period}`);
      console.error();
      console.error("If you need to extend the end_height for this chain hash, manually edit");
      console.error("the existing override in constants.toml instead of adding a new one.");
      process.exit(1);
    }
  }

  // Create the new override
  const newOverride: ChainHashOverride = {
    start_height: startHeight,
    end_height: endHeight,
    chain_hash: chainHash,
    grace_period: options.gracePeriod,
  };

  console.log();
  console.log("New override to add:");
  console.log(`  start_height: ${newOverride.start_height}`);
  console.log(`  end_height: ${newOverride.end_height}`);
  console.log(`  chain_hash: ${newOverride.chain_hash}`);
  console.log(`  grace_period: ${newOverride.grace_period}`);
  console.log();

  // Generate the updated content
  const updatedContent = addOverrideToToml(content, newOverride, existingOverrides);

  if (options.dryRun) {
    const allOverrides = [...existingOverrides, newOverride];
    console.log("=== DRY RUN - Changes that would be made ===");
    console.log();
    console.log("New CHAIN_HASH_OVERRIDES value:");
    console.log("---");
    console.log(`CHAIN_HASH_OVERRIDES = ${buildOverridesArrayString(allOverrides)}`);
    console.log("---");
    console.log();
    console.log("No changes written to file.");
  } else {
    fs.writeFileSync(constantsPath, updatedContent);
    console.log(`Successfully updated ${constantsPath}!`);
  }
}

main().catch((error) => {
  console.error("Unexpected error:", error);
  process.exit(1);
});
