// Chat search paints matches with the CSS Custom Highlight API, so rendered
// Markdown stays untouched; virtual rows are repainted as they appear.
const supported = () =>
  typeof CSS !== "undefined" && "highlights" in CSS && typeof Highlight === "function";

const searches = new WeakMap();

function ranges(root, query) {
  const needle = query.toLocaleLowerCase();
  const found = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.data.toLocaleLowerCase();
    for (let at = text.indexOf(needle); at !== -1; at = text.indexOf(needle, at + needle.length)) {
      const range = new Range();
      range.setStart(node, at);
      range.setEnd(node, at + needle.length);
      found.push(range);
    }
  }
  return found;
}

function paint(results, state) {
  state.frame = 0;
  if (!supported()) return;
  const all = [], current = [];
  if (state.query) {
    for (const row of results.querySelectorAll("[data-transcript-row]")) {
      const found = ranges(row, state.query);
      (row.dataset.transcriptRow === state.current ? current : all).push(...found);
    }
  }
  CSS.highlights.set("chat-search", new Highlight(...all));
  CSS.highlights.set("chat-search-current", new Highlight(...current));
  // The jump aligns the row; a long message still needs its match in view.
  if (state.reveal && current.length) {
    if (results.hasAttribute("data-transcript-adjusting")) {
      schedule(results, state);
      return;
    }
    state.reveal = false;
    const match = current[0].getBoundingClientRect();
    const view = results.getBoundingClientRect();
    // The composer covers the lower part of the panel.
    if (match.top < view.top || match.bottom > view.top + view.height * 0.7)
      current[0].startContainer.parentElement?.scrollIntoView({ block: "center" });
  }
}

function schedule(results, state) {
  if (!state.frame) state.frame = requestAnimationFrame(() => paint(results, state));
}

/** Highlights `query` in rendered rows; `currentId` marks the selected match. */
export function paintChatSearch(results, query, currentId) {
  let state = searches.get(results);
  if (!state) {
    state = { frame: 0, query: "", current: "", reveal: false };
    state.observer = new MutationObserver(() => schedule(results, state));
    state.scroll = () => schedule(results, state);
    searches.set(results, state);
  }
  if (!state.query && query) {
    state.observer.observe(results, { childList: true, subtree: true, characterData: true });
    results.addEventListener("scroll", state.scroll, { passive: true });
  }
  state.reveal = currentId !== state.current || query !== state.query;
  state.query = query;
  state.current = currentId;
  if (!query) {
    state.observer.disconnect();
    results.removeEventListener("scroll", state.scroll);
  }
  schedule(results, state);
}
