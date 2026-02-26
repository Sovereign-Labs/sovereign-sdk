import { Command } from "commander";
import { createAddChainhashOverrideCommand } from "./upgrade/add-chainhash-override";

async function main(): Promise<void> {
  const program = new Command();

  program
    .name("sov-cli")
    .description("Sovereign SDK CLI utilities")
    .showHelpAfterError();

  const upgradeCommand = program
    .command("upgrade")
    .description("Upgrade-related operations");

  upgradeCommand.addCommand(createAddChainhashOverrideCommand());

  if (process.argv.length <= 2) {
    program.help();
  }

  await program.parseAsync(process.argv);
}

main().catch((error) => {
  console.error(error instanceof Error ? `Error: ${error.message}` : error);
  process.exit(1);
});
