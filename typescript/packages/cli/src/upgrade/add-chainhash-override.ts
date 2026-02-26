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

interface AddChainhashOverrideOptions {
  constantsPath: string;
  url: string;
  blocksFromNow?: number;
  endHeight?: number;
  gracePeriod: number;
  dryRun: boolean;
}

function parseIntegerOption(value: string, optionName: string): number {
  const parsed = Number.parseInt(value, 10);
  if (Number.isNaN(parsed)) {
    throw new Error(`Invalid value for ${optionName}: "${value}"`);
  }
  return parsed;
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

  const overrideRegex = /^CHAIN_HASH_OVERRIDES\s*=\s*\[.*\]$/m;
  if (overrideRegex.test(content)) {
    return content.replace(overrideRegex, overrideLine);
  }

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

    if (inConstantsSection && trimmedLine.startsWith("[")) {
      if (!addedOverride) {
        result.push(overrideLine);
        addedOverride = true;
      }
      inConstantsSection = false;
    }

    result.push(line);

    if (
      inConstantsSection &&
      !addedOverride &&
      trimmedLine.startsWith("CHAIN_NAME")
    ) {
      result.push(overrideLine);
      addedOverride = true;
    }
  }

  if (!addedOverride) {
    result.push("");
    result.push(overrideLine);
  }

  return result.join("\n");
}

async function runAddChainhashOverride(
  options: AddChainhashOverrideOptions
): Promise<void> {
  if (options.blocksFromNow === undefined && options.endHeight === undefined) {
    throw new Error(
      "At least one of --blocks-from-now or --end-height is required"
    );
  }

  const constantsPath = path.resolve(options.constantsPath);
  if (!fs.existsSync(constantsPath)) {
    throw new Error(`File not found: ${constantsPath}`);
  }

  console.log(`Connecting to rollup at ${options.url}...`);

  let chainHash: string;
  let rollupHeight: number;

  try {
    [chainHash, rollupHeight] = await Promise.all([
      fetchChainHash(options.url),
      fetchRollupHeight(options.url),
    ]);
  } catch (error) {
    throw new Error(
      `Error connecting to rollup: ${error instanceof Error ? error.message : error}`
    );
  }

  console.log(`Current chain hash: ${chainHash}`);
  console.log(`Current rollup height: ${rollupHeight}`);
  console.log();

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

  const content = fs.readFileSync(constantsPath, "utf-8");
  const parsed = parseConstantsToml(content);

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

    const existingWithSameHash = existingOverrides.find(
      (override) => override.chain_hash === chainHash
    );
    if (existingWithSameHash) {
      throw new Error(
        [
          "This chain hash is already present in CHAIN_HASH_OVERRIDES.",
          "",
          "Existing override with this hash:",
          `  start_height: ${existingWithSameHash.start_height}`,
          `  end_height: ${existingWithSameHash.end_height}`,
          `  chain_hash: ${existingWithSameHash.chain_hash}`,
          `  grace_period: ${existingWithSameHash.grace_period}`,
          "",
          "If you need to extend the end_height for this chain hash, manually edit",
          "the existing override in constants.toml instead of adding a new one.",
        ].join("\n")
      );
    }
  }

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
    return;
  }

  fs.writeFileSync(constantsPath, updatedContent);
  console.log(`Successfully updated ${constantsPath}!`);
}

export function createAddChainhashOverrideCommand(): Command {
  return new Command("add-chainhash-override")
    .alias("add-chain-hash-override")
    .description(
      "Add a chain hash override to constants.toml for rollup upgrades"
    )
    .requiredOption("-c, --constants-path <path>", "Path to constants.toml file")
    .option("-u, --url <url>", "Rollup node URL", "http://localhost:12346")
    .option(
      "-b, --blocks-from-now <blocks>",
      "Number of blocks from current height until upgrade",
      (value: string) => parseIntegerOption(value, "--blocks-from-now")
    )
    .option(
      "-e, --end-height <height>",
      "Explicit end height for the override",
      (value: string) => parseIntegerOption(value, "--end-height")
    )
    .option(
      "-g, --grace-period <blocks>",
      "Grace period in blocks after end_height. During this time, both chain hashes will be accepted.",
      (value: string) => parseIntegerOption(value, "--grace-period"),
      100
    )
    .option("-d, --dry-run", "Preview changes without writing to file", false)
    .action(async (options: AddChainhashOverrideOptions) => {
      await runAddChainhashOverride(options);
    });
}
