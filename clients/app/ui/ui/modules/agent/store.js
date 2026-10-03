// One draft for all agent settings pages: switching pages keeps unsaved edits
// and one save applies them together.
import { buildRequest, changes, draftFromSnapshot } from "./draft.js";

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
    emit({ loading: true, feedback: null });
    loading = service
      .read()
      .then(
        (snapshot) =>
          emit({
            snapshot,
            draft: draftFromSnapshot(snapshot),
            errors: {},
            loading: false,
          }),
        (error) =>
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
  },
});
