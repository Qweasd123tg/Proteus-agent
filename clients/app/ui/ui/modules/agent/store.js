// One draft for all agent settings pages: switching pages keeps unsaved edits
// and one save applies them together.
import { buildRequest, changes, draftFromSnapshot, rebaseDraft } from "./draft.js";
import { revisionDraft } from "./revisions.js";

const listeners = new Set();
let state = {
  snapshot: null,
  draft: null,
  errors: {},
  loading: false,
  saving: false,
  feedback: null,
};
let loading;
let refreshRevision = 0, pendingRefresh;

function emit(next) {
  state = { ...state, ...next };
  for (const listener of listeners) listener(state);
}

export const agentSettings = Object.freeze({
  state: () => state,
  subscribe(listener, signal) {
    listeners.add(listener);
    listener(state);
    signal?.addEventListener("abort", () => listeners.delete(listener), {
      once: true,
    });
    return () => listeners.delete(listener);
  },
  /** The first page loads the profile; later pages reuse the same draft. */
  load(service, force = false) {
    if (loading) return loading;
    if (state.snapshot && !force) return Promise.resolve(state);
    const revision = ++refreshRevision;
    emit({ loading: true, feedback: null });
    loading = service
      .read()
      .then(
        (snapshot) => revision === refreshRevision &&
          emit({
            snapshot,
            draft: draftFromSnapshot(snapshot),
            errors: {},
            loading: false,
          }),
        (error) => revision === refreshRevision &&
          emit({
            loading: false,
            feedback: { kind: "error", text: `Не удалось загрузить профиль: ${error.message}` },
          }),
      )
      .finally(() => (loading = undefined));
    return loading;
  },
  update(change) {
    if (!state.draft || state.saving) return;
    const draft = structuredClone(state.draft);
    change(draft);
    emit({ draft, feedback: null });
  },
  async refresh(service, error = null) {
    if (state.saving) { pendingRefresh = service; return; }
    const revision = ++refreshRevision;
    try {
      const snapshot = await service.read();
      if (revision !== refreshRevision) return;
      if (state.saving) { pendingRefresh = service; return; }
      error ??= snapshot.warnings?.find(warning => warning.severity === "error")?.message ?? null;
      if (!error && !state.loading && JSON.stringify(state.snapshot) === JSON.stringify(snapshot)) return;
      const dirty = state.snapshot && state.draft && changes(state.snapshot,state.draft).size > 0;
      const draft = dirty ? rebaseDraft(state.snapshot,snapshot,state.draft) : draftFromSnapshot(snapshot);
      emit({ snapshot, draft, loading: false, errors: dirty ? state.errors : {}, feedback: error
        ? {kind:"error", text:`Профиль не обновлён: ${error}`}
        : dirty ? {kind:"warning",text:"Профиль изменён извне · ваши несохранённые изменения сохранены"} : null });
    } catch (error) {
      if (revision===refreshRevision) emit({feedback:{kind:"error",text:`Не удалось обновить профиль: ${error.message}`}});
    }
  },
  setError(key, message) {
    const errors = { ...state.errors };
    if (message) errors[key] = message;
    else delete errors[key];
    emit({ errors });
  },
  changes: () =>
    state.snapshot && state.draft ? changes(state.snapshot, state.draft) : new Set(),
  reset() {
    if (!state.snapshot || state.saving) return;
    emit({ draft: draftFromSnapshot(state.snapshot), errors: {}, feedback: null });
  },
  /** Puts a recorded profile state into the draft; saving applies it. */
  restore(revisionState) {
    if (!state.snapshot || state.saving) return;
    emit({ draft: revisionDraft(state.snapshot, revisionState), errors: {}, feedback: null });
  },
  async save(service) {
    if (!state.snapshot || state.saving || Object.keys(state.errors).length) return;
    let request;
    try {
      request = buildRequest(state.snapshot, state.draft);
    } catch (error) {
      emit({ feedback: { kind: "error", text: `Исправьте JSON: ${error.message}` } });
      return;
    }
    emit({ saving: true, feedback: null });
    ++refreshRevision;
    try {
      const snapshot = await service.save(request);
      emit({
        snapshot,
        draft: draftFromSnapshot(snapshot),
        errors: {},
        saving: false,
        feedback: { kind: "saved", text: "Сохранено · новые запросы используют обновлённую сборку" },
      });
    } catch (error) {
      emit({
        saving: false,
        feedback: { kind: "error", text: `Не сохранено: ${error.message}` },
      });
    }
    if (pendingRefresh) { const service = pendingRefresh; pendingRefresh = undefined; await agentSettings.refresh(service); }
  },
});
