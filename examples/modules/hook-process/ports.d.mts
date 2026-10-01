import type { Handler, HookContext } from "./hooks.mjs";
type PortContext = HookContext & { readonly hasUI: false };
interface PiCall {
  readonly type: "tool_call";
  readonly toolCallId: string;
  readonly toolName: string;
  readonly input: Readonly<Record<string, unknown>>;
}
interface TextPart { readonly type: "text"; readonly text: string }
interface PiResult extends Omit<PiCall, "type"> {
  readonly type: "tool_result";
  readonly content: readonly TextPart[];
  readonly details: unknown;
  readonly isError: boolean;
}
export function piToolCall(handler: (event: PiCall, ctx: PortContext) =>
  void | { block?: boolean; reason?: string } | Promise<void | { block?: boolean; reason?: string }>): Handler<"before_tool">;
export function piToolResult(handler: (event: PiResult, ctx: PortContext) =>
  void | { content?: readonly TextPart[]; details?: unknown; isError?: boolean } |
  Promise<void | { content?: readonly TextPart[]; details?: unknown; isError?: boolean }>): Handler<"after_tool">;
interface OpenCodeInput { readonly tool: string; readonly callID: string; readonly sessionID: string | null }
export function openCodeToolBefore(handler: (input: OpenCodeInput, output: { args: Record<string, unknown> }) => void | Promise<void>): Handler<"before_tool">;
export function openCodeToolAfter(handler: (input: OpenCodeInput & { readonly args: Readonly<Record<string, unknown>> }, output: { output: string }) => void | Promise<void>): Handler<"after_tool">;
export function preToolUseDecision(result: { code: number; stdout: string; stderr: string }): ReturnType<Handler<"before_tool">>;
