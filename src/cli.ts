#!/usr/bin/env node
import { runSearch } from "./search.js";
import { runMcp } from "./mcp.js";

const [sub, ...rest] = process.argv.slice(2);

if (sub === "search") {
  await runSearch(rest);
} else if (sub === "mcp") {
  await runMcp(rest);
} else {
  const help =
    "Usage: statespace <command> [options]\n\n" +
    "Commands:\n" +
    "  search <query>   Search indexed documentation\n" +
    "  mcp              Start the MCP server\n\n" +
    "Run statespace <command> --help for command options.\n";
  if (sub) {
    process.stderr.write(`Unknown command: ${sub}\n\n${help}`);
    process.exit(1);
  }
  process.stdout.write(help);
}
