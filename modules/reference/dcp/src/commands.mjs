import { handleStatsCommand } from "../node_modules/@tarquinen/opencode-dcp/lib/commands/stats.ts";
import { handleContextCommand } from "../node_modules/@tarquinen/opencode-dcp/lib/commands/context.ts";
import { handleDecompressCommand } from "../node_modules/@tarquinen/opencode-dcp/lib/commands/decompress.ts";

export const command = { name: "dcp", description: "DCP: статистика, контекст и восстановление блоков", arguments: "stats|context|decompress [NUMBER]" };
export const commandSpec = { name: "dcp", description: command.description,
  input_schema: {type: "object", properties: {arguments: {type: "string"}}, required: ["arguments"], additionalProperties: false},
  surface: {kind: "function", strict: false, output_schema: null}, safety: "ReadOnly",
  supports_parallel_tool_calls: false, timeout_ms: 30000, metadata: {upstream: "opencode-dcp@3.2.0"} };

export async function executeDcpCommand(argumentsText, context) {
  const [name, ...args] = argumentsText.trim().split(/\s+/);
  const output = [];
  context.client.commandMessage = (text) => output.push(text);
  if (name === "decompress") await handleDecompressCommand({ ...context, args });
  else if (name === "stats" || name === "context") {
    if (args.length) throw new Error(`/dcp ${name} does not accept arguments`);
    await (name === "stats" ? handleStatsCommand : handleContextCommand)(context);
  } else if (!name || name === "help") output.push("/dcp stats\n/dcp context\n/dcp decompress [NUMBER]");
  else throw new Error(`unsupported DCP command: ${name}`);
  return output.join("\n");
}
