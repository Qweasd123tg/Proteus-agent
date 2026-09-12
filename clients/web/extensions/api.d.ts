/** Client extension contract v1. Independent of agent process-module contracts. */
export interface ExtensionManifest {
  apiVersion: 1;
  id: string;
  name: string;
  description: string;
  entry: string;
  requires: string[];
  /** A panel adds an independent resizable, collapsible column; widgets share the overview. */
  presentation?: 'widget' | 'panel';
  /** Independent entry; loaded only by the explicit Configure action. */
  settings?: { entry: string; requires: string[] };
}

export interface ExtensionStorage {
  get(key: string): string | null;
  set(key: string, value: string): void;
  remove(key: string): void;
  /** Same-client changes for this extension; caller releases the subscription. */
  subscribe(callback: () => void): () => void;
}

export interface ExtensionPane {
  root: ShadowRoot;
  show(): void;
  hide(): void;
}

export interface ExtensionContext {
  /** Panel-owned root; inherited design tokens, no Leptos or Tauri dependency. */
  root: ShadowRoot;
  /** Compact content inside the host's interactive button; absent for a settings entry. */
  compact?: ShadowRoot;
  /** Host actions, independent of a particular extension id; absent in settings. */
  panel?: { open(): void; move(location: 'left' | 'right'): void };
  /** Transient independent columns owned by this mount; absent in settings. No additional services or authority.
   * Columns may be open together. Location inserts left/right of chat without replacing it.
   * Width is client-stored by pane id; other owned state lasts for the session.
   * Collapse and moving preserve roots; owner disposal releases every pane.
   */
  panels?: { create(id: string, options: { title: string; location: 'left' | 'right' }): ExtensionPane };
  /** Only declared interfaces; each interface defines its own data contract. */
  services: Readonly<Record<string, unknown>>;
  storage: ExtensionStorage;
  /** Aborts on disable, removal, retry, mount failure, session change or client unmount. Collapse, moving and SPA navigation preserve the instance. */
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

/** Public GET /usage?session_dir=…; null means no canonical journal.
 * Usage fields are provider-reported totals per exchange, not streaming deltas.
 * Cache categories belong to input; reasoning belongs to output.
 * Other peer sessions have their own journals and reports.
 */
export interface AgentUsageReader {
  read(): Promise<SessionUsageSnapshot | null>;
}

export interface SessionUsageSnapshot {
  session_id: string;
  revision: number;
  latest_turn_id: string | null;
  requests: Array<{
    exchange_id: string;
    turn_id: string | null;
    model: { provider: string; model: string };
    origin: 'direct' | 'compactor';
    started_at_ms: number;
    finished_at_ms: number | null;
    status: 'unfinished' | 'completed' | 'error' | 'canceled' | 'timeout';
    finish_reason: string | null;
    usage: {
      input_tokens: number;
      output_tokens: number;
      cached_input_tokens: number | null;
      cache_creation_input_tokens: number | null;
      reasoning_output_tokens: number | null;
    } | null;
    message_count: number;
    tool_count: number;
    reasoning_effort: string | null;
    max_output_tokens: number | null;
  }>;
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

/** agent.session.read: client projection, not a second HTTP poller. */
export interface AgentSessionReader {
  read(): SessionView;
  subscribe(callback: (snapshot: SessionView) => void): () => void;
}
export interface SessionView {
  session_dir: string | null;
  workspace: string;
  model: string;
  mode: string;
  reasoning: string;
  status: string;
  events: number;
  tools: number;
  pending: number;
  plan: Array<{ step: string; status: string }>;
  context: { used: number; max: number; trigger: number | null } | null;
}
/** agent.workspace.read: explicit session-scoped, read-only public HTTP surface. */
export interface AgentWorkspaceReader {
  list(path: string): Promise<{
    path: string;
    entries: Array<{ name: string; path: string; kind: 'directory' | 'file' | 'symlink' | 'special' }>;
    truncated: boolean;
  }>;
  read(path: string): Promise<{ path: string; size: number; kind: 'text' | 'binary' | 'too_large'; text: string | null }>;
  /** Read-only Git changes relative to HEAD, within the addressed workspace. */
  changes(): Promise<{
    repository: boolean;
    entries: Array<{ path: string; status: 'added' | 'modified' | 'deleted' | 'renamed' | 'untracked' | 'conflict' }>;
    truncated: boolean;
  }>;
  diff(path: string): Promise<{ path: string; kind: 'text' | 'binary' | 'too_large' | 'unavailable'; patch: string | null }>;
}
