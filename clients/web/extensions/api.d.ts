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

/** Optional agent.model.quota.read service; unmodified GET /model/quota.
 * null means unsupported, never unlimited. Percentages may exceed 100.
 * Timestamps use Unix seconds. observed_at is provider fetch time, including cache hits.
 */
export interface AgentModelQuotaReader {
  read(): Promise<ModelQuotaSnapshot | null>;
}

export interface ModelQuotaSnapshot {
  observed_at: number;
  plan: string | null;
  buckets: Array<{
    id: string;
    name: string | null;
    allowed: boolean | null;
    limit_reached: boolean | null;
    windows: Array<{
      id: string;
      used_percent: number;
      duration_seconds: number | null;
      resets_at: number | null;
    }>;
  }>;
  credits: { available: boolean; unlimited: boolean; balance: string | null } | null;
}
