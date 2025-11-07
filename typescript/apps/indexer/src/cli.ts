import { Rollup, SovereignClient } from "@sovereign-sdk/web3";
import { hideBin } from "yargs/helpers";
import yargs from "yargs/yargs";
import packageJson from "../package.json";
import { getDefaultDatabase } from "./db";
import { Indexer } from "./indexer";
import logger from "./logger";

let _indexer: Indexer | undefined = undefined;

function scriptName() {
  return "sov-indexer";
}

interface CliArgs {
  rollupUrl: string;
}

const argv = yargs(hideBin(process.argv))
  .scriptName(scriptName())
  .option("rollup-url", {
    describe: "URL for the rollup",
    type: "string",
    demandOption: true,
  })
  .help()
  .version(`${scriptName()} version: ${packageJson.version}`)
  .parse() as CliArgs;

async function onExit() {
  logger.info("Exit signal received, shutting down indexer");

  await _indexer?.stop();
}

process.on("SIGINT", onExit);
process.on("SIGTERM", onExit);
process.on("SIGHUP", onExit);

const rollup = new Rollup(
  {
    client: new SovereignClient.SovereignSDK({ baseURL: argv.rollupUrl }),
    // biome-ignore lint/suspicious/noExplicitAny: types arent used
    serializer: {} as any,
    context: {},
  },
  // biome-ignore lint/suspicious/noExplicitAny: types arent used
  {} as any,
);
_indexer = new Indexer({
  rollup,
  database: getDefaultDatabase(),
});

_indexer.run().catch(console.error);
