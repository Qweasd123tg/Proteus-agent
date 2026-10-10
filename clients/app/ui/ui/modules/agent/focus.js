// Cross-page navigation inside agent settings: the host selects the page and
// the page reveals the requested element once it is built. Draft state lives in
// the shared store, so switching pages keeps unsaved edits.
const revealers = new Map();
let pending;

/** Called by a page after each build; consumes a request made before it existed. */
export function onReveal(page, reveal, signal) {
  revealers.set(page, reveal);
  signal.addEventListener("abort", () => revealers.get(page) === reveal && revealers.delete(page), { once: true });
  if (pending?.page === page) {
    const { target } = pending;
    pending = undefined;
    requestAnimationFrame(() => reveal(target));
  }
}

/** Shows an agent settings page and the element matching `target` (a selector). */
export function openAgentTarget(page, target) {
  pending = { page, target };
  document.dispatchEvent(new CustomEvent("proteus-select-settings-module", { detail: page }));
  const reveal = revealers.get(page);
  if (reveal && pending) {
    pending = undefined;
    requestAnimationFrame(() => reveal(target));
  }
}

/** Opens enclosing details, scrolls to the element, focuses and marks it. */
export function spotlight(element) {
  if (!element) return;
  for (let parent = element.parentElement?.closest("details"); parent; parent = parent.parentElement?.closest("details"))
    parent.open = true;
  element.scrollIntoView({ block: "center", behavior: "instant" });
  if (!element.matches("button,input,select,textarea,summary,[tabindex]")) element.tabIndex = -1;
  element.focus({ preventScroll: true });
  element.classList.add("agent-spotlight");
  setTimeout(() => element.classList.remove("agent-spotlight"), 1600);
}
