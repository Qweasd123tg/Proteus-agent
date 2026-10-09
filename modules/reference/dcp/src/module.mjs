import { tool } from "@opencode-ai/plugin";
import { createCompressMessageTool, createCompressRangeTool } from "../node_modules/@tarquinen/opencode-dcp/lib/compress/index.ts";
import { attachCompressionDuration } from "../node_modules/@tarquinen/opencode-dcp/lib/compress/state.ts";
import { createSessionState, ensureSessionInitialized, checkSession, syncToolCache } from "../node_modules/@tarquinen/opencode-dcp/lib/state/index.ts";
import { assignMessageRefs } from "../node_modules/@tarquinen/opencode-dcp/lib/message-ids.ts";
import { countTokens } from "../node_modules/@tarquinen/opencode-dcp/lib/token-utils.ts";
import { buildPriorityMap, buildToolIdList, injectCompressNudges, injectMessageIds, prune, stripHallucinations, syncCompressionBlocks } from "../node_modules/@tarquinen/opencode-dcp/lib/messages/index.ts";
import { createSystemPromptHandler } from "../node_modules/@tarquinen/opencode-dcp/lib/hooks.ts";
import { configuration } from "./config.mjs";
import { project } from "./messages.mjs";
import { PromptStore } from "./platform/prompts.mjs";
import { Logger } from "./platform/logger.mjs";
import { transaction, saveSessionState } from "./platform/persistence.mjs";
import { command, commandSpec, executeDcpCommand } from "./commands.mjs";

export function createDcp(settings = {}) {
  const { config, directory } = configuration(settings);
  const prompts = new PromptStore();
  const logger = new Logger(config.debug);
  const sessions = new Map();
  const queues = new Map();
  const define = (state, client) => (config.compress.mode === "message" ? createCompressMessageTool : createCompressRangeTool)({ config, prompts, logger, state, client });
  const definition = define(createSessionState("compact"), {});
  const schema = tool.schema.object(definition.args);
  const spec = { name: "compress", description: definition.description, input_schema: tool.schema.toJSONSchema(schema),
    surface: { kind: "function", strict: false, output_schema: null }, safety: "ReadOnly", supports_parallel_tool_calls: false,
    timeout_ms: 30000, metadata: { permission: "compress", upstream: "opencode-dcp@3.2.0" } };

  async function serial(session, signal, operation) {
    const previous = queues.get(session) ?? Promise.resolve();
    const running = previous.catch(() => {}).then(async () => {
      signal.throwIfAborted();
      const state = structuredClone(sessions.get(session) ?? createSessionState("compact"));
      const result = await transaction(directory, signal, () => operation(state));
      sessions.set(session, state);
      return result;
    });
    queues.set(session, running);
    try { return await running; }
    finally { if (queues.get(session) === running) queues.delete(session); }
  }

  async function load(state, conversation, session, model) {
    const raw = project(conversation.messages, conversation.messages, session, model, conversation.model_context).messages;
    const client = { session: {
      messages: async () => ({ data: raw }),
      get: async () => ({ data: {} }), // No OpenCode parent-session namespace in this port.
    } };
    await ensureSessionInitialized(client, state, session, logger, raw, false);
    // Repeated real compaction uses different canonical checkpoint identities.
    const compaction = raw.findLast((message) => message.info.summary)?.info.id;
    if (state.checkpointId && state.checkpointId !== compaction) state.lastCompaction = 0;
    state.checkpointId = compaction;
    await checkSession(client, state, logger, raw, false);
    state.compressPermission = "allow"; // ToolRegistry/policy already governs exposure and invocation.
    assignMessageRefs(state, raw);
    return { raw, client };
  }

  return {
    spec,
    tools: [{spec, model_visible: true, user_command: null},
      {spec: commandSpec, model_visible: false, user_command: command}],
    async hook(input, signal) {
      const event = input.event;
      if (event.event !== "before_model" || !input.attribution.agent) return { action: "continue" };
      if (!input.conversation) throw new Error("DCP hook needs a conversation snapshot");
      const session = input.attribution.agent.session_id;
      return serial(session, signal, async (state) => {
        const request = event.request;
        const { raw } = await load(state, input.conversation, session, request.model);
        state.modelContextLimit = request.limits.max_input_tokens ?? undefined;
        state.systemPromptTokens = countTokens(request.instructions.map((instruction) => instruction.text).join("\n"));
        state.compressPermission = request.tools.some((tool) => tool.name === "compress") ? "allow" : "deny";
        const view = project(request.messages, input.conversation.messages, session, request.model, input.conversation.model_context);
        stripHallucinations(view.messages, state.idFormat);
        assignMessageRefs(state, view.messages);
        syncCompressionBlocks(state, logger, raw);
        syncToolCache(state, config, logger, view.messages);
        buildToolIdList(state, view.messages);
        prune(state, logger, config, view.messages, view.summaryBase);
        const priorities = buildPriorityMap(config, state, view.messages);
        injectCompressNudges(state, config, logger, view.messages, prompts.getRuntimePrompts(), priorities, event.origin === "direct");
        injectMessageIds(state, config, view.messages, priorities);
        const system = { system: request.instructions.map((instruction) => instruction.text) };
        await createSystemPromptHandler(state, logger, config, prompts)({ sessionID: session,
          model: { limit: { context: state.modelContextLimit ?? 0 } } }, system);
        const instructions = system.system.map((text, index) => ({ ...(request.instructions[index] ?? { kind: "System", priority: 100 }), text }));
        await saveSessionState(state, logger);
        return { action: "model_context", messages: view.restore(), instructions };
      });
    },
    async invoke(input, readConversation, signal) {
      if (input.call.name === "dcp") {
        const args = input.call.args;
        if (!args || Object.keys(args).length !== 1 || typeof args.arguments !== "string") throw new Error("invalid DCP command arguments");
        const snapshot = await readConversation("host.conversation.snapshot");
        signal.throwIfAborted();
        return serial(snapshot.session_id, signal, async (state) => {
          const {client, raw} = await load(state, snapshot.conversation, snapshot.session_id);
          syncCompressionBlocks(state, logger, raw);
          syncToolCache(state, config, logger, raw);
          buildToolIdList(state, raw);
          const output = await executeDcpCommand(args.arguments, {client, state, logger, sessionId: snapshot.session_id, messages: raw});
          signal.throwIfAborted();
          return {call_id: input.call.id, ok: true, output, content: [], error: null, metadata: null};
        });
      }
      if (input.call.name !== "compress") throw new Error("unknown DCP tool");
      if (!input.attribution.agent) throw new Error("compress requires an agent conversation");
      const args = schema.parse(input.call.args);
      const snapshot = await readConversation("host.conversation.read");
      signal.throwIfAborted();
      const session = input.attribution.agent.session_id;
      return serial(session, signal, async (state) => {
        const { client, raw } = await load(state, snapshot.conversation, session);
        syncToolCache(state, config, logger, raw);
        buildToolIdList(state, raw);
        const started = Date.now();
        const output = await define(state, client).execute(args, {
          sessionID: session, messageID: snapshot.message_id, callID: input.call.id,
          agent: "proteus", directory: input.cwd, worktree: input.cwd, abort: signal,
          ask: async () => signal.throwIfAborted(), metadata: () => {},
        });
        signal.throwIfAborted();
        attachCompressionDuration(state.prune.messages, snapshot.message_id, input.call.id, Date.now() - started);
        await saveSessionState(state, logger);
        return { call_id: input.call.id, ok: true, output, content: [], error: null, metadata: null };
      });
    },
  };
}
