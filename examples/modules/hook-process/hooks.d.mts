export interface HookContext {
  readonly cwd: string;
  readonly attribution: {
    readonly execution_id: string;
    readonly agent: null | { readonly session_id: string; readonly thread_id: string; readonly turn_id: string };
  };
  readonly config: Readonly<Record<string, unknown>>;
  readonly signal: AbortSignal;
}
export interface ToolCall {
  readonly id: string;
  readonly name: string;
  readonly args: Readonly<Record<string, unknown>>;
  readonly surface: "function" | "freeform";
  readonly raw_arguments: string | null;
}
export interface ToolResult {
  readonly call_id: string;
  readonly ok: boolean;
  readonly output: string;
  readonly content: readonly unknown[];
  readonly error: string | null;
  readonly metadata: unknown;
}
export interface ModelRequest {
  readonly messages: readonly unknown[];
  readonly instructions: readonly unknown[];
  readonly [field: string]: unknown;
}
export interface HookEvents {
  turn_started: { readonly event: "turn_started"; readonly task: unknown; readonly history: readonly unknown[] };
  before_model: { readonly event: "before_model"; readonly origin: string; readonly request: ModelRequest };
  before_tool: { readonly event: "before_tool"; readonly call: ToolCall; readonly spec: unknown; readonly blocked: string | null };
  after_tool: { readonly event: "after_tool"; readonly call: ToolCall; readonly result: ToolResult };
  turn_settled: { readonly event: "turn_settled"; readonly status: "success" | "error" | "canceled" | "timeout"; readonly output: unknown; readonly error: string | null };
}
export type HookResponse =
  | { action: "continue" }
  | { action: "block_tool"; reason: string }
  | { action: "tool_output"; output: string }
  | { action: "model_context"; messages: readonly unknown[]; instructions: readonly unknown[] };
type Continue = Extract<HookResponse, { action: "continue" }>;
export type EventResponse = {
  turn_started: Continue;
  before_model: Continue | Extract<HookResponse, { action: "model_context" }>;
  before_tool: Continue | Extract<HookResponse, { action: "block_tool" }>;
  after_tool: Continue | Extract<HookResponse, { action: "tool_output" }>;
  turn_settled: Continue;
};
export type Handler<E extends keyof HookEvents> =
  (event: HookEvents[E], ctx: HookContext) => EventResponse[E] | void | Promise<EventResponse[E] | void>;
export interface Hooks {
  readonly config: Readonly<Record<string, unknown>>;
  on<E extends keyof HookEvents>(event: E, handler: Handler<E>, options?: { tools?: readonly string[] }): void;
}
export function createHooks(settings?: Record<string, unknown>): {
  api: Hooks;
  seal(): void;
  invoke(input: { event: HookEvents[keyof HookEvents]; cwd: string; attribution: HookContext["attribution"] }, signal: AbortSignal): Promise<{ result: HookResponse }>;
};
