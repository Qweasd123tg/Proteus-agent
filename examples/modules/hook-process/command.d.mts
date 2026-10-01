export function runCommand(
  command: string, args: readonly string[], input: unknown,
  ctx: { readonly cwd: string; readonly signal: AbortSignal },
): Promise<{ code: number; stdout: string; stderr: string }>;
