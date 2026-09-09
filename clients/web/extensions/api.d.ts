/** Client extension contract v1. Independent of agent process-module contracts. */
export interface ExtensionManifest {
  apiVersion: 1;
  id: string;
  name: string;
  description: string;
  entry: string;
  requires: string[];
}

export interface ExtensionStorage {
  get(key: string): string | null;
  set(key: string, value: string): void;
  remove(key: string): void;
}

export interface ExtensionContext {
  /** Panel-owned root; inherited design tokens, no Leptos or Tauri dependency. */
  root: ShadowRoot;
  /** Only declared interfaces; each interface defines its own data contract. */
  services: Readonly<Record<string, unknown>>;
  storage: ExtensionStorage;
  /** Aborts on disable, collapse, removal, mount failure or client unmount. */
  signal: AbortSignal;
}

/** Mount must settle. Release custom subscriptions/timers in the disposer.
 * Bind DOM listeners and fetch to signal, including while mounting asynchronously.
 */
export type Mount = (context: ExtensionContext) => void | (() => void) | Promise<void | (() => void)>;

/** The current web client exposes this optional interface under agent.config.read.
 * Result is the unmodified JSON response of the public Proteus GET /config API.
 */
export interface AgentConfigReader {
  read(): Promise<Record<string, unknown>>;
}
