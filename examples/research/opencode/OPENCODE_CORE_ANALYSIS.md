# packages/opencode: Detailed Core Analysis

## Thesis

`packages/opencode` is not “the CLI package”.

It is the backend runtime of the whole system:

- it owns the HTTP API
- it owns the session model
- it owns message persistence
- it owns the agent loop
- it owns tool execution
- it owns permission gating
- it owns snapshotting and revert/diff
- it owns provider/model abstraction
- it owns plugins, MCP, ACP, workspaces, and sync/replay

The CLI, TUI, web app, desktop app, SDK clients, IDE integrations, and Slack integration are all different entry surfaces into that backend.

## The Short Version

If you compress OpenCode into one sentence, it looks like this:

`user input -> session/message creation -> agent/model/tool resolution -> model stream -> processor turns stream into persisted parts -> bus broadcasts state -> clients render the result`

The most important thing is that OpenCode stores the agent turn as structured state, not just as a blob of text.

The assistant response is decomposed into parts:

- `text`
- `reasoning`
- `tool`
- `file`
- `patch`
- `step-start`
- `step-finish`
- `subtask`
- `compaction`

That decision explains most of the architecture.

## Why This Is Server Core, Not a CLI

The CLI entrypoint in [packages/opencode/src/index.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/index.ts) mainly does bootstrap work:

- parses commands with `yargs`
- initializes logs
- performs one-time DB migration
- registers commands like `run`, `serve`, `web`, `session`, `agent`, `mcp`, `acp`, `plugin`

That file is the shell around the runtime, not the runtime itself.

The actual backend starts appearing in these files:

- [packages/opencode/src/effect/app-runtime.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/effect/app-runtime.ts)
- [packages/opencode/src/server/server.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/server.ts)
- [packages/opencode/src/session/prompt.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/prompt.ts)
- [packages/opencode/src/session/processor.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/processor.ts)
- [packages/opencode/src/tool/registry.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/tool/registry.ts)
- [packages/opencode/src/permission/permission.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/permission/permission.ts)

The CLI is just one client of this core.

## Runtime Shape: Effect Service Graph

The best “system diagram in code” is [packages/opencode/src/effect/app-runtime.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/effect/app-runtime.ts).

`AppLayer` merges the services that make the system work:

- config
- auth
- storage
- bus
- provider
- agent
- skill
- permission
- question
- session
- session status
- session run state
- session processor
- session prompt
- tool registry
- LSP
- MCP
- plugin
- project/VCS/worktree
- PTY
- installation/share services

This is important because OpenCode is not written as “a few modules that call each other directly”.

It is written as a runtime graph of services with explicit scoping and dependency injection.

That makes it possible to:

- host multiple project instances in one server
- reuse the same core through CLI, TUI, web, desktop, ACP, and SDK
- keep long-lived state in scoped services instead of globals

## Instance Model: The Core Is Scoped Per Directory

OpenCode’s runtime is instance-scoped, not process-global.

The key files are:

- [packages/opencode/src/project/instance.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/project/instance.ts)
- [packages/opencode/src/effect/instance-state.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/effect/instance-state.ts)
- [packages/opencode/src/project/bootstrap.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/project/bootstrap.ts)

What this means in practice:

- an “instance” is bound to a working directory and worktree
- services can cache state per instance directory
- the server can host more than one project/workspace
- async callbacks can restore the correct project context through `Instance.bind` and `Instance.restore`

`InstanceBootstrap` initializes the services that make a project “live”:

- plugins
- LSP
- formatter
- file service
- file watcher
- VCS
- snapshot service
- share service

This is backend bootstrapping, not UI bootstrapping.

## HTTP/API Layer

The server is assembled in [packages/opencode/src/server/server.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/server.ts).

It mounts three main surfaces:

- control-plane routes
- instance routes
- UI routes

The instance API is defined in [packages/opencode/src/server/instance/index.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/instance/index.ts).

That API includes:

- sessions
- providers
- permissions
- questions
- files
- project metadata
- PTY
- MCP
- sync
- events
- TUI control

So the HTTP server is not a thin wrapper over CLI commands.

It is the transport layer for the actual backend model.

## Workspace Routing and Remote Forwarding

OpenCode can route requests locally or forward them to another workspace.

The key file is [packages/opencode/src/server/instance/middleware.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/instance/middleware.ts).

That middleware does three jobs:

- resolves the working directory from query/header
- decides whether the request should stay local or be forwarded
- proxies HTTP and WebSocket traffic to remote workspace targets when necessary

The remote sync/replay side lives in [packages/opencode/src/control-plane/workspace.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/control-plane/workspace.ts) and [packages/opencode/src/server/proxy.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/proxy.ts).

This is another strong signal that `packages/opencode` is backend infrastructure. A pure CLI package would not need request routing, remote replay, or workspace proxies.

## Data Model: Session, Message, Part

The most important conceptual model in OpenCode is:

- `session`
- `message`
- `part`

The schema is split across:

- [packages/opencode/src/session/session.sql.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/session.sql.ts)
- [packages/opencode/src/session/message-v2.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/message-v2.ts)

The DB tables are:

- `session`
- `message`
- `part`
- `todo`
- `session_entry`
- `permission`

The crucial design choice is that a message is not stored as one string.

Instead:

- a user message contains metadata like `agent`, `model`, `system`, `tools`, `format`
- an assistant message contains `agent`, `providerID`, `modelID`, `cost`, `tokens`, `finish`, `structured`, `error`
- the visible and invisible content is split into typed parts

Important part types:

- `text`
- `reasoning`
- `tool`
- `file`
- `patch`
- `snapshot`
- `step-start`
- `step-finish`
- `subtask`
- `compaction`
- `retry`
- `agent`

This gives OpenCode several properties:

- streaming text can be persisted incrementally
- tool calls can have proper lifecycle state
- diffs can be attached to the assistant turn
- subagent invocations are first-class state
- compaction and retry are part of the conversation model
- the UI can render a structured timeline instead of parsing text heuristically

## Persistence Is Event-Sourced Through SyncEvent

OpenCode does not update session state only through ad hoc SQL writes.

The backbone is:

- [packages/opencode/src/sync/sync-event.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/sync/sync-event.ts)
- [packages/opencode/src/session/projectors.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/projectors.ts)
- [packages/opencode/src/server/projectors.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/projectors.ts)

The model is:

1. code emits a sync event
2. the sync system assigns a sequence number per aggregate
3. a projector writes the materialized state into SQLite
4. the bus emits the live event to subscribers
5. the global bus can also fan it out outside the local instance

This is why session updates, message updates, and part updates all look like event emissions.

It gives OpenCode:

- deterministic replay
- workspace restore
- sequence-safe syncing
- separation between write intent and materialized DB rows

This is a backend pattern, not a command-line scripting pattern.

## SQLite Layer

The DB core is in [packages/opencode/src/storage/db.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/storage/db.ts).

Important details:

- SQLite runs in WAL mode
- migrations are applied at startup
- database side effects can be deferred until after transactions
- transactions are bound to instance context
- sync events use immediate transactions to keep sequence handling safe

There is also an older file-based storage service in [packages/opencode/src/storage/storage.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/storage/storage.ts), mainly for legacy/migration-compatible resources such as diff/session artifacts.

The current center of gravity is SQLite plus sync-event projection.

## Session Service: The Canonical State API

The session service is in [packages/opencode/src/session/session.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/session.ts).

It provides the canonical API for:

- creating sessions
- forking sessions
- touching/updating metadata
- setting permissions
- storing revert info
- listing children
- removing sessions
- updating messages
- updating parts
- reading messages
- diff lookup

The key thing is that `Session` does not directly behave like a chat-memory helper.

It behaves like a state service over an event-sourced session aggregate.

The `fork` implementation is especially revealing:

- it creates a fresh child session
- copies messages up to a boundary
- remaps message IDs
- remaps assistant parent references
- recreates parts under new IDs

That means branching conversations are a real data model feature, not a UI illusion.

## SessionRunState: Concurrency Control

[packages/opencode/src/session/run-state.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/run-state.ts) enforces a very important invariant:

- one active runner per session

This service:

- tracks active runners by `sessionID`
- exposes `ensureRunning`
- exposes `cancel`
- exposes a separate `startShell` path
- updates `session.status`

This prevents overlapping model loops for the same session.

Without this layer, the backend would become race-prone very quickly.

## SessionPrompt: The Real Orchestration Layer

If you only read one file, read [packages/opencode/src/session/prompt.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/prompt.ts).

This is the file that turns a user-facing prompt into the running agent loop.

It does far more than “send messages to a model”.

### What SessionPrompt Actually Owns

It owns:

- user message construction
- agent selection
- model/variant resolution
- prompt/file/resource expansion
- synthetic system reminders
- tool surface resolution
- subtask handling
- shell execution path
- slash command expansion
- structured output enforcement
- main loop control
- title/summary background jobs
- compaction triggers

### Input Normalization

`createUserMessage` is important because it resolves prompt parts before the model sees them.

It can transform:

- local files
- directories
- MCP resources
- inline data URLs
- explicit `@agent` mentions

into structured parts plus synthetic text explaining what was attached or read.

That means context ingestion is explicit and persisted, not magical.

### Command Path

`command(...)` is another signal that this is backend orchestration.

Commands are not special UI behavior.

They are templates that get expanded into:

- normal prompt parts
- or a `subtask` part that instructs the backend to spawn a child agent session

So slash commands are really alternate routes into the same session engine.

### Shell Path

`shellImpl(...)` makes a user-invoked shell command appear inside the same message-part model:

- it creates a user message
- it creates an assistant message
- it creates a running `tool` part for `bash`
- it streams shell output into tool metadata/output
- it finalizes the assistant/tool state the same way the agent loop does

That is a backend-centric design: shell activity becomes part of session history.

## The Main Agent Loop

The central loop is `runLoop(...)` in `SessionPrompt`.

Conceptually it does this:

1. load compacted session history
2. find the latest user message and latest assistant state
3. detect pending subtasks or compaction tasks
4. resolve the selected agent and model
5. inject reminders for plan mode or agent switching
6. create a fresh assistant message
7. create a `SessionProcessor` handle
8. resolve the tool map
9. build system prompts and model messages
10. call `processor.process(...)`
11. decide whether to stop, continue, or compact and loop again

Important behavior:

- first step may asynchronously generate a session title
- summaries can be kicked off in parallel
- compaction is scheduled automatically on context overflow
- if the agent has a max-step limit, an extra reminder is injected on the last step
- structured JSON output adds a synthetic mandatory tool

This is not a thin AI call wrapper. It is a loop controller for an agent runtime.

## SessionProcessor: Stream-to-State Machine

[packages/opencode/src/session/processor.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/processor.ts) is the second most important file.

Its job is to turn an AI SDK event stream into persistent session state.

### It Tracks Live Turn State

The processor keeps a context that includes:

- assistant message under construction
- active tool calls
- current snapshot hash
- blocked state
- compaction state
- current text part
- current reasoning parts

### It Handles Event Types

It reacts to streamed events like:

- `reasoning-start`
- `reasoning-delta`
- `reasoning-end`
- `tool-input-start`
- `tool-call`
- `tool-result`
- `tool-error`
- `start-step`
- `finish-step`
- `text-start`
- `text-delta`
- `text-end`

### Tool Call State Machine

Tool calls move through explicit states:

- `pending`
- `running`
- `completed`
- `error`

This is persisted as `ToolPart.state`, not inferred afterwards.

That enables:

- streaming UI updates
- retry/abort-safe cleanup
- exact permission linkage to a tool call
- postmortem inspection of tool behavior

### Doom Loop Detection

The processor guards against repeating the same tool call forever.

If the last few tool invocations are identical, it asks the special `doom_loop` permission.

That is a very backend-oriented safeguard. It is not a UI prompt hack.

### Snapshot and Patch Emission

Before the stream starts, it captures a snapshot baseline.

On step finish or cleanup, it computes the patch from that baseline.

If files changed, it emits a `patch` part.

This is how OpenCode can later reason about diffs and revert operations as part of session state.

### Completion Semantics

The processor does not just “collect text until done”.

It decides whether the loop result is:

- `continue`
- `stop`
- `compact`

That return value feeds back into `SessionPrompt.runLoop`.

## Tools: OpenCode’s Real Capability Surface

The tool system is built in [packages/opencode/src/tool/registry.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/tool/registry.ts).

It combines:

- built-in tools
- local user-defined tools from config directories
- plugin-provided tools
- MCP-provided tools

Built-in tools include:

- `bash`
- `read`
- `glob`
- `grep`
- `edit`
- `write`
- `apply_patch`
- `task`
- `todo`
- `skill`
- `webfetch`
- `websearch`
- `codesearch`
- `question`
- `lsp`
- `plan`

Important details:

- tool definitions are transformed before exposure
- plugins can mutate tool definitions
- the available surface depends on model family
- for some GPT-family models, `apply_patch` is preferred over edit/write
- tool descriptions for `task` and `skill` are dynamically expanded with available agent/skill info

So “tool availability” is not static. It is runtime-computed.

## TaskTool: Subagents Are Child Sessions

[packages/opencode/src/tool/task.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/tool/task.ts) is critical if you want to build your own agent runtime.

OpenCode’s subagent model is not fake.

When the model invokes `task`:

- a child session is created
- the selected subagent gets its own permission envelope
- the subagent runs through the same `SessionPrompt` machinery
- the tool result stores the child `sessionId`
- the parent sees the child result as tool output

This is much stronger than “invoke another prompt template”.

It means subagents are real branches in session state.

## Permission Engine

The permission core is in:

- [packages/opencode/src/permission/permission.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/permission/permission.ts)
- [packages/opencode/src/permission/evaluate.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/permission/evaluate.ts)

Rules are simple:

- `allow`
- `ask`
- `deny`

But the system around them is not simple.

### Important Behavior

- rule evaluation is wildcard-based
- the last matching rule wins
- project-level approved permissions are stored in `PermissionTable`
- `ask(...)` emits a bus event and waits on a deferred reply
- `reject` can cascade to all pending requests in the same session
- corrected rejection can return user feedback back into the agent flow

This means permission is not just prompt text.

It is a runtime control plane for side effects.

## Question System

OpenCode has a parallel interaction primitive for structured user clarification in [packages/opencode/src/question/index.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/question/index.ts).

It supports:

- one or more questions
- short headers
- options with descriptions
- multiple choice
- optional custom answer

Like permissions, questions are:

- emitted as events
- stored as pending requests
- resolved asynchronously

This is part of the backend interaction model, not only UI logic.

## Agents: Prompt + Permissions + Role

The agent core is in [packages/opencode/src/agent/agent.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/agent/agent.ts).

This file shows that an OpenCode agent is defined by:

- name
- description
- mode
- model override
- variant
- prompt
- generation options
- step limit
- permission ruleset

Built-in agents are:

- `build`
- `plan`
- `general`
- `explore`
- hidden `compaction`
- hidden `title`
- hidden `summary`

The important conceptual point:

OpenCode agents are not just prompt presets.

They are execution profiles.

`plan` is especially revealing:

- it is a primary agent
- it is permission-restricted
- in plan mode only the plan file is editable
- the backend injects explicit workflow reminders into the user message

So “mode” in OpenCode is implemented by runtime policy plus orchestration, not just by tone or prompt wording.

## Providers and Models

The provider abstraction is in [packages/opencode/src/provider/provider.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/provider/provider.ts).

This service does several jobs at once:

- loads provider inventory
- resolves model metadata
- resolves actual AI SDK language models
- handles provider-specific auth and options
- chooses default and small models
- supports custom discovery and custom model loaders

Important details:

- model metadata is normalized into a common `Model` shape
- costs and token limits are stored centrally
- provider-specific SDK resolution is hidden behind one interface
- some providers use chat API, others responses API, others special workflow models
- the default model can come from config, recent history, or provider inventory
- small-model selection is a separate policy path for lightweight tasks like titles

This is backend-grade model routing, not a hardcoded “call OpenAI” integration.

## LLM Layer: The Provider Adapter

[packages/opencode/src/session/llm.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/llm.ts) is where the session engine meets the AI SDK.

It is responsible for:

- resolving the actual language model object
- composing the final system prompt stack
- applying plugin hooks to system, params, and headers
- merging model options, provider options, agent options, and variant overrides
- converting tool defs into AI SDK tools
- dealing with provider-specific quirks

Important quirks handled here:

- OpenAI OAuth path uses provider instructions differently
- workflow models get special tool-execution plumbing
- LiteLLM-like proxies may need a dummy no-op tool if history already contains tool calls

That file is the transport adapter from OpenCode’s internal state model into provider-specific model execution.

## Plugins: Deep Runtime Hooks

The plugin core is in [packages/opencode/src/plugin/plugin.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/plugin/plugin.ts) and [packages/opencode/src/plugin/loader.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/plugin/loader.ts).

Plugins are not an afterthought.

They can hook into:

- chat system prompt transformation
- chat params
- chat headers
- chat messages
- tool definition shaping
- tool execute before/after
- shell env
- config changes
- generic event stream

This means a plugin can affect core execution, not just decorate UI behavior.

That is one of the strongest reasons to treat `packages/opencode` as a platform runtime.

## MCP: External Tools and Resources as First-Class Inputs

The MCP integration in [packages/opencode/src/mcp/mcp.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/mcp/mcp.ts) is also backend-native.

It manages:

- stdio/SSE/HTTP MCP transports
- auth and OAuth
- server connection state
- tool discovery
- prompt discovery
- resource discovery
- wrapping MCP tools into AI SDK tools

In `SessionPrompt`, MCP resources can also be read and transformed into message parts before the model runs.

So MCP is integrated both as:

- external tool surface
- external context source

## Snapshotting, Patch Generation, Revert

The snapshot engine is in [packages/opencode/src/snapshot/snapshot.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/snapshot/snapshot.ts).

This subsystem quietly does a lot:

- keeps a shadow git repository per project/worktree
- stages allowed changes into that snapshot repo
- computes patch hashes and changed file lists
- supports restore/revert/diff operations
- ignores gitignored files correctly
- limits large snapshot capture

This gives OpenCode reliable:

- diff summaries
- revert support
- per-step patch emission
- “what changed during this turn?” introspection

Again: this is backend state management.

## Bus and Event Streaming

The local bus is in [packages/opencode/src/bus/bus.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/bus/bus.ts).

The SSE route is in [packages/opencode/src/server/instance/event.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/instance/event.ts).

This is how the rest of the system sees state changes:

- internal services publish bus events
- the event route exposes them as SSE
- UI and SDK clients subscribe and mirror state
- global bus also sees cross-instance events

This is why the web app and TUI can be relatively thin.

The backend already emits a rich event model.

## ACP Exists Because This Is Already a Backend

The ACP implementation in [packages/opencode/src/acp/README.md](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/acp/README.md) is not a random add-on.

It works because `packages/opencode` already provides:

- a session model
- a prompt orchestration API
- a tool registry
- file operations
- a runtime context

ACP is basically another transport/control surface on top of the same core.

## What Makes This Architecture Strong

The strongest design choices are:

- typed message parts instead of raw transcript strings
- event-sourced writes with replay
- instance-scoped runtime state
- explicit permission engine
- subagents as child sessions
- backend-owned diff/snapshot model
- plugin hooks at the execution layer
- one backend serving many clients

These choices scale much better than the common pattern:

- UI sends prompt
- model returns text
- tool output is appended ad hoc
- history is just a transcript array

OpenCode is significantly more structured than that.

## What Makes It Complex

The main sources of complexity are:

- a lot of state is split across services instead of one controller
- event sourcing plus projection plus bus plus global bus can be hard to trace at first
- `SessionPrompt` is doing many jobs and is very dense
- provider quirks are centralized, which is powerful but cognitively heavy
- workspace forwarding adds another axis of routing on top of instance scoping

So the architecture is strong, but the learning curve is real.

## The Real Core Files

If you want only the minimum set that explains the backend, read these:

1. [packages/opencode/src/effect/app-runtime.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/effect/app-runtime.ts)
2. [packages/opencode/src/server/server.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/server.ts)
3. [packages/opencode/src/server/instance/index.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/server/instance/index.ts)
4. [packages/opencode/src/project/instance.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/project/instance.ts)
5. [packages/opencode/src/session/session.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/session.ts)
6. [packages/opencode/src/session/message-v2.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/message-v2.ts)
7. [packages/opencode/src/session/prompt.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/prompt.ts)
8. [packages/opencode/src/session/processor.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/session/processor.ts)
9. [packages/opencode/src/tool/registry.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/tool/registry.ts)
10. [packages/opencode/src/tool/task.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/tool/task.ts)
11. [packages/opencode/src/permission/permission.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/permission/permission.ts)
12. [packages/opencode/src/provider/provider.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/provider/provider.ts)
13. [packages/opencode/src/plugin/plugin.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/plugin/plugin.ts)
14. [packages/opencode/src/sync/sync-event.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/sync/sync-event.ts)
15. [packages/opencode/src/snapshot/snapshot.ts](/home/qweasd123tg/Code%20/Agent%20/Analys/opencode/source/opencode/packages/opencode/src/snapshot/snapshot.ts)

## Practical Design Lessons If You Build Your Own Agent

If your goal is “build my own general-purpose agent runtime”, the main lessons from OpenCode are:

- build a backend first, not a chat UI first
- model a turn as structured state, not as plain text
- separate orchestration from provider transport
- make tool execution explicit and stateful
- treat permissions as runtime control, not just prompt text
- make subagents real sessions or real branches of state
- emit events that clients can subscribe to instead of letting every client invent its own state machine
- own filesystem diff/revert at the backend layer
- keep model/provider abstraction separate from agent logic

## Bottom Line

The shortest accurate description of `packages/opencode` is:

It is a multi-tenant, instance-scoped, event-driven agent backend with a CLI attached to it.

The CLI is the shell.

The real product is the runtime inside `packages/opencode`.
