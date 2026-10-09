import { systemPrompt } from "../../node_modules/@tarquinen/opencode-dcp/lib/prompts/system.ts";
import { rangePrompt } from "../../node_modules/@tarquinen/opencode-dcp/lib/prompts/compress-range.ts";
import { messagePrompt } from "../../node_modules/@tarquinen/opencode-dcp/lib/prompts/compress-message.ts";
import { CONTEXT_LIMIT_NUDGE } from "../../node_modules/@tarquinen/opencode-dcp/lib/prompts/context-limit-nudge.ts";
import { TURN_NUDGE } from "../../node_modules/@tarquinen/opencode-dcp/lib/prompts/turn-nudge.ts";
import { ITERATION_NUDGE } from "../../node_modules/@tarquinen/opencode-dcp/lib/prompts/iteration-nudge.ts";
import { MANUAL_MODE_SYSTEM_EXTENSION, SUBAGENT_SYSTEM_EXTENSION } from "../../node_modules/@tarquinen/opencode-dcp/lib/prompts/extensions/system.ts";

export class PromptStore {
  constructor(format = "compact") {
    this.prompts = {
      system: systemPrompt(format), compressRange: rangePrompt(format), compressMessage: messagePrompt(format),
      contextLimitNudge: CONTEXT_LIMIT_NUDGE, turnNudge: TURN_NUDGE, iterationNudge: ITERATION_NUDGE,
      manualExtension: MANUAL_MODE_SYSTEM_EXTENSION, subagentExtension: SUBAGENT_SYSTEM_EXTENSION,
    };
  }
  reload() {}
  getRuntimePrompts() { return this.prompts; }
}
