import { defaultConfig, mergeLayer, getInvalidConfigKeys, validateConfigTypes } from "../node_modules/@tarquinen/opencode-dcp/lib/config.ts";
import { homedir } from "node:os";
import { join, isAbsolute } from "node:path";

export function configuration(input = {}) {
  if (!input || typeof input !== "object" || Array.isArray(input)) throw new Error("DCP config must be an object");
  const { state_dir, ...options } = input;
  const supported = ["debug", "turnProtection", "protectedFilePatterns", "compress", "strategies"];
  for (const key of Object.keys(options)) if (!supported.includes(key)) throw new Error(`unsupported DCP setting: ${key}`);
  const unknown = getInvalidConfigKeys(options);
  const invalid = validateConfigTypes(options);
  if (unknown.length || invalid.length) throw new Error(`invalid DCP config: ${JSON.stringify({ unknown, invalid })}`);
  if (options.compress?.permission !== undefined || options.compress?.showCompression !== undefined) {
    throw new Error("compress permission belongs to Proteus policy; OpenCode showCompression is unsupported");
  }
  const config = mergeLayer(structuredClone(defaultConfig), options);
  config.autoUpdate = false;
  config.commands.enabled = false;
  config.pruneNotification = "off";
  // Same tool-name aliases used by upstream V2, plus explicitly selected Proteus tools.
  for (const tools of [config.compress.protectedTools, config.commands.protectedTools,
    config.strategies.deduplication.protectedTools, config.strategies.purgeErrors.protectedTools]) {
    for (const [from, to] of [["task", "subagent"], ["bash", "shell"], ["apply_patch", "patch"]]) {
      if (tools.includes(from) && !tools.includes(to)) tools.push(to);
    }
  }
  const directory = state_dir ?? join(process.env.XDG_DATA_HOME || join(homedir(), ".local/share"), "Proteus-agent/modules/dcp");
  if (typeof directory !== "string" || !isAbsolute(directory)) throw new Error("state_dir must be an absolute path");
  return { config, directory };
}
