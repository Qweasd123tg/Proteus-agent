/** Client extension contract v2. Independent of agent process-module contracts. */
export interface ExtensionManifest {
  apiVersion: 2;
  id: string;
  name: string;
  description: string;
  icon?: string;
  /** Static illustration; resolved relative to the manifest without executing any view. */
  preview?: { src: string; alt: string };
  views: ExtensionView[];
}

export interface ExtensionView {
  /** Only compact and workspace may share one view instance. */
  surfaces: ClientSurface[];
  entry: string;
  requires: string[];
  layout: 'scroll' | 'fill' | 'form' | 'editor';
  isolation: 'shadow' | 'light';
}

/** Source and menu placement belong to the host, never to the manifest. */
export interface ExtensionRecord {
  id: string;
  source: 'builtin' | 'package';
  enabled: boolean;
  manifest?: ExtensionManifest;
  error?: string;
  /** Builtins only; installed packages always appear under Extensions. */
  settingsGroup?: 'agent' | 'builtin';
  required?: boolean;
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
  /** Aborts when this tab is closed or its owner is disposed. */
  signal: AbortSignal;
  show(): void;
  hide(): void;
  /** Close and release an owned tab. Idempotent after disposal. */
  close(): void;
}

export interface ExtensionContext {
  /** View-owned root; isolation follows the view descriptor. No Leptos or Tauri dependency. */
  root: ShadowRoot | HTMLElement;
  surface: ClientSurface;
  /** A combined workspace/compact view mounts once and declares both here. */
  surfaces: readonly ClientSurface[];
  /** Compact content inside the host's interactive button; absent for a settings entry. */
  compact?: ShadowRoot;
  /** Live detail text shown with the compact icon's hover label; host owns presentation. */
  hover?: { set(text: string): void };
  /** Host actions, independent of a particular extension id; absent in settings. */
  panel?: { open(): void; move(location: 'left' | 'right'): void };
  /** Tabs owned by this mount; no additional services or authority. Switching and hiding preserve roots.
   * Closing releases the tab and calls onClose; owner disposal releases every tab.
   * Location is a host placement hint; the Proteus tab workspace groups both sides together.
   */
  panels?: { create(id: string, options: { title: string; location?: 'left' | 'right'; onClose?: () => void }): ExtensionPane };
  /** Only declared interfaces; each interface defines its own data contract. */
  services: Readonly<Record<string, unknown>>;
  storage: ExtensionStorage;
  /** Aborts on disable, removal, retry, mount failure, session change, demo close or client unmount. Collapse, moving and SPA navigation preserve the instance.
   * A demo ("Попробовать" on a disabled package) passes demo services with fictional data and in-memory storage. */
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

/** Optional agent.config.builder service; the public GET/POST /config/builder.
 * read() returns the saved profile snapshot: slots with their implementations,
 * providers, permission modes, hooks, tools and opaque module_config objects.
 * save() sends the complete selection; the server builds and validates the
 * assembly before it writes the profile and returns the new snapshot.
 * history() is GET /config/history: states replaced by earlier saves, newest
 * first. Saving a recorded state through save() rolls the profile back.
 */
export interface AgentConfigBuilder {
  read(): Promise<Record<string, unknown>>;
  save(request: {
    modules: Record<string, string>;
    hooks: string[];
    module_config: Record<string, Record<string, Record<string, unknown>>>;
    tools_enabled: string[];
    active_provider: string | null;
    permission_mode: string | null;
  }): Promise<Record<string, unknown>>;
  history(): Promise<{
    revisions: Array<{
      id: string;
      /** Unix milliseconds of the save that replaced this state. */
      replaced_at_ms: number;
      state: {
        active_provider: string | null;
        permission_mode: string;
        active_modules: Array<{ slot: string; id: string }>;
        hooks: string[];
        module_config: Record<string, Record<string, Record<string, unknown>>>;
        tools_enabled: string[];
      };
    }>;
  }>;
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

/** Settings are ordered pages; composer slots select one enabled implementation. */
export type ClientSurface = 'compact' | 'workspace' | 'settings' | 'composer-model' | 'composer-access';
export interface ClientPreferences {
  fontSize: number; chatWidth: number; animations: boolean;
  autoScroll: boolean; toolCardsCollapsed: boolean; notifications: boolean; sendMode: 'enter' | 'ctrl-enter';
}
export interface ClientComposerState {
  model: string; models: Array<{name:string;label:string;hidden:boolean}>;
  reasoning: boolean; effort: string; effortLabel: string; efforts: string[];
  mode: 'normal' | 'auto' | 'plan';
  modes: Array<{value: string; label: string; description: string}>;
}
/** client.preferences and client.composer. Writes validate before dispatch; failures throw.
 * Composer changes use the existing agent commands; snapshots reflect their completion.
 * Service operations are valid only while the mount signal is active.
 */
export interface ClientStateService<State, Writable = State> {
  read(): State;
  set<K extends keyof Writable>(key: K, value: Writable[K]): void;
  subscribe(callback: () => void): () => void;
}
export type ClientPreferencesService = ClientStateService<ClientPreferences>;
export type ClientComposerService = ClientStateService<ClientComposerState, Pick<ClientComposerState,'model'|'effort'|'mode'>>;
export interface ClientDiagnosticsService {
  mount(view: 'usage'|'analysis'|'architecture', root: HTMLElement): () => void;
}
export interface ClientModulesService { mount(root: HTMLElement): () => void }
