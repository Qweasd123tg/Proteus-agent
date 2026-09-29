import { node } from "./dom.js";
import { enableHorizontalReorder } from "./horizontal-reorder.js";
import {
  initialLayout,
  parseLayout,
  layoutKey,
  groupOf,
  moveTab,
  closeTab,
  mergeGroups,
} from "../ui/workspace/state.mjs";
import { createGroup, createTab } from "../ui/workspace/group.js";
import { watchViewMotion } from "../ui/view-motion.js";
import { popoverMotion } from "../ui/popover-motion.js";
import { tabMotion } from "../ui/workspace/tab-motion.js";

// The board owns placement; providers own content and its lifetime.
export function createWorkspace(target, { storage } = {}) {
  const controller = new AbortController(),
    { signal } = controller;
  const element = node("div", null, "tab-workspace"),
    surfaces = node("div", null, "workspace-tab-content workspace-surfaces");
  const picker = node("div", null, "workspace-picker");
  picker.setAttribute("popover", "auto");
  picker.setAttribute("aria-label", "Открыть вкладку");
  const handle = node("div", null, "workspace-resize");
  handle.tabIndex = 0;
  handle.setAttribute("role", "separator");
  handle.setAttribute("aria-label", "Размер областей");
  handle.setAttribute("aria-orientation", "vertical");
  const status = node("span", null, "workspace-status");
  status.setAttribute("role", "status");
  const sources = new Map(),
    tabs = new Map(),
    scrolls = new WeakMap(),
    visibility = new WeakMap(),
    viewMotion = new Map();
  let records = [],
    layout,
    drag,
    pickerGroup = 0;
  try {
    layout = parseLayout(storage?.getItem(layoutKey));
  } catch (error) {
    layout = initialLayout();
    status.textContent = `${error.message}. Используется новая раскладка.`;
  }
  const groups = [0, 1].map((index) =>
    createGroup(index, {
      pick: showPicker,
      transfer,
      signal,
      focus(index) {
        focus(index);
        const id = groups[index].tabs
          .querySelector("[aria-selected=true]")
          ?.closest("[data-tab-id]").dataset.tabId;
        record(id)?.onSelect?.();
      },
    }),
  );
  element.append(
    groups[0].element,
    handle,
    groups[1].element,
    surfaces,
    picker,
    status,
  );
  target.append(element);
  const pickerMotion = popoverMotion(picker);
  const animateTabs = tabMotion(element, signal);
  function save() {
    try {
      storage?.setItem(layoutKey, JSON.stringify(layout));
    } catch {
      status.textContent = "Не удалось сохранить раскладку";
    }
  }
  function focus(index) {
    layout.focused = index;
    for (let i = 0; i < 2; i++)
      groups[i].element.classList.toggle("focused", i === index);
  }
  function visibleIds(index) {
    return (
      layout.groups[index]?.ids.filter((id) =>
        records.some((r) => r.id === id && !r.collapsed),
      ) || []
    );
  }
  function record(id) {
    return records.find((r) => r.id === id);
  }
  function owner(id) {
    return [...sources.values()].find((s) =>
      s.records.some((r) => r.id === id),
    );
  }
  function choose(id, index = pickerGroup) {
    picker.hidePopover();
    if (groupOf(layout, id) < 0) moveTab(layout, id, index);
    owner(id)?.select?.(id);
    reveal(id);
  }
  function reveal(id) {
    if (!record(id)) return;
    if (groupOf(layout, id) < 0) moveTab(layout, id, layout.focused);
    const index = groupOf(layout, id);
    if (layout.groups[index].active !== id)
      layout.groups[index].previous = layout.groups[index].active;
    layout.groups[index].active = id;
    focus(index);
    render();
    save();
    record(id)?.onSelect?.();
    tabs.get(id)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }
  function close(id) {
    const index = groupOf(layout, id);
    owner(id)?.close?.(id);
    closeTab(layout, id);
    if (layout.groups.length === 2 && !visibleIds(index).length)
      mergeGroups(layout);
    render();
    save();
    const active = groups[layout.focused].tabs.querySelector(
      "[aria-selected=true]",
    );
    active?.focus({ preventScroll: true });
    record(active?.closest("[data-tab-id]").dataset.tabId)?.onSelect?.();
  }
  function split() {
    if (layout.groups.length === 2) mergeGroups(layout);
    else layout.groups.push({ ids: [], active: "" });
    render();
    save();
  }
  function transfer(index) {
    const id = groups[index].tabs
      .querySelector("[aria-selected=true]")
      ?.closest("[data-tab-id]").dataset.tabId;
    if (!id) return;
    if (layout.groups.length === 1) layout.groups.push({ ids: [], active: "" });
    moveTab(layout, id, 1 - index);
    render();
    save();
    tabs.get(id)?.querySelector("[role=tab]").focus();
    record(id)?.onSelect?.();
  }
  function choices(container) {
    container.replaceChildren();
    for (const r of records.filter((r) => !r.owned || r.client)) {
      const b = node("button", r.manifest?.name ?? r.id);
      b.type = "button";
      b.dataset.openTab = r.id;
      container.append(b);
    }
  }
  function showPicker(index, add) {
    pickerGroup = index;
    choices(picker);
    const r = add.getBoundingClientRect();
    picker.style.left = `${Math.max(8, Math.min(innerWidth - 300, r.right - 280))}px`;
    picker.style.top = `${Math.max(8, Math.min(innerHeight - 340, r.bottom + 6))}px`;
    pickerMotion.show();
  }
  function render() {
    const finishTabMotion = animateTabs();
    records = [...sources.values()].flatMap((s) => s.records);
    for (const [root, stop] of viewMotion)
      if (!records.some((r) => r.element === root)) {
        stop();
        viewMotion.delete(root);
      }
    for (const r of records)
      if (r.element && !viewMotion.has(r.element))
        viewMotion.set(
          r.element,
          watchViewMotion(r.element, { signal, inPlace: true }),
        );
    const available = new Set(
      records.filter((r) => !r.collapsed).map((r) => r.id),
    );
    for (const r of records)
      if (!r.collapsed && groupOf(layout, r.id) < 0)
        layout.groups[0].ids.push(r.id);
    for (const [id, tab] of tabs)
      if (!available.has(id)) {
        tab.remove();
        tabs.delete(id);
      }
    for (let index = 0; index < 2; index++) {
      const view = groups[index],
        group = layout.groups[index];
      view.element.hidden = !group;
      if (!group) continue;
      const ids = visibleIds(index),
        active = ids.includes(group.active) ? group.active : ids[0] || "";
      view.empty.hidden = !!active;
      choices(view.empty);
      view.move.disabled = !active;
      for (const [position, id] of ids.entries()) {
        const r = record(id);
        let tab = tabs.get(id);
        if (!tab) {
          tab = createTab(r);
          tabs.set(id, tab);
        }
        const name = tab.firstElementChild,
          selected = id === active,
          title = r.manifest?.name || id;
        if (name.textContent !== title) {
          name.textContent = title;
          name.title = title;
        }
        tab.lastElementChild.setAttribute("aria-label", `Закрыть: ${title}`);
        name.setAttribute("aria-selected", String(selected));
        name.tabIndex = selected ? 0 : -1;
        tab.classList.toggle("active", selected);
        if (view.tabs.children[position] !== tab)
          view.tabs.insertBefore(tab, view.tabs.children[position] ?? null);
      }
      for (const r of records.filter((r) => groupOf(layout, r.id) === index)) {
        const root = r.element;
        if (!root) continue;
        const selected = r.id === active,
          wasHidden = root.hidden,
          moving = root.parentNode !== surfaces;
        const focused = root.contains(document.activeElement)
          ? document.activeElement
          : null;
        // WebKit can reset the surface offset on hide/reveal. Nested scroll
        // containers retain their own offsets; do not scan the transcript.
        if (!wasHidden && (!selected || moving))
          scrolls.set(root, [root.scrollTop, root.scrollLeft]);
        if (!selected && !wasHidden) {
          root.dispatchEvent(new Event("module-hide"));
          for (const details of root.querySelectorAll(
            "details.composer-menu[open]",
          ))
            details.open = false;
        }
        if (root.hidden !== !selected) root.hidden = !selected;
        if (root.inert !== !selected) root.inert = !selected;
        if (moving) surfaces.append(root);
        if (root.dataset.workspaceGroup !== String(index))
          root.dataset.workspaceGroup = index;
        if (selected && (wasHidden || moving)) {
          const saved = scrolls.get(root);
          if (saved) {
            root.scrollTop = saved[0];
            root.scrollLeft = saved[1];
          }
          if (focused) focused.focus({ preventScroll: true });
        }
        if (root.id !== `workspace-view-${r.id}`)
          root.id = `workspace-view-${r.id}`;
        if (root.getAttribute("role") !== "tabpanel")
          root.setAttribute("role", "tabpanel");
        if (root.getAttribute("aria-labelledby") !== `workspace-tab-${r.id}`)
          root.setAttribute("aria-labelledby", `workspace-tab-${r.id}`);
      }
    }
    for (const r of records)
      if (groupOf(layout, r.id) < 0 && r.element) {
        if (!r.element.hidden)
          r.element.dispatchEvent(new Event("module-hide"));
        r.element.hidden = true;
        r.element.inert = true;
        if (r.element.parentNode !== surfaces) surfaces.append(r.element);
      }
    for (const r of records)
      if (r.element && visibility.get(r.element) !== !r.element.hidden) {
        visibility.set(r.element, !r.element.hidden);
        r.onVisibility?.(!r.element.hidden);
      }
    handle.hidden = layout.groups.length === 1;
    element.classList.toggle("split", layout.groups.length === 2);
    ratio(layout.ratio);
    handle.setAttribute("aria-valuemin", "20");
    handle.setAttribute("aria-valuemax", "80");
    for (const b of document.querySelectorAll("[data-workspace-split]")) {
      b.setAttribute("aria-pressed", String(layout.groups.length === 2));
      b.title =
        layout.groups.length === 2 ? "Объединить области" : "Разделить область";
      b.setAttribute("aria-label", b.title);
    }
    focus(layout.focused);
    finishTabMotion();
  }
  for (const [index, group] of groups.entries()) {
    enableHorizontalReorder(group.tabs, {
      itemSelector: ".workspace-tab",
      handleSelector: ".workspace-tab-name",
      id: (tab) => tab.dataset.tabId,
      signal,
      lists: () =>
        groups.filter((_, i) => i < layout.groups.length).map((g) => g.tabs),
      commit(id, before, target) {
        moveTab(layout, id, Number(target.dataset.group), before);
        render();
        save();
      },
    });
    group.tabs.addEventListener(
      "click",
      (e) => {
        const tab = e.target.closest("[data-tab-id]");
        if (!tab) return;
        if (e.target.closest(".workspace-tab-close")) close(tab.dataset.tabId);
        else reveal(tab.dataset.tabId);
      },
      { signal },
    );
    group.tabs.addEventListener(
      "keydown",
      (e) => {
        if (
          !e.target.matches("[role=tab]") ||
          e.altKey ||
          e.ctrlKey ||
          e.metaKey
        )
          return;
        const ids = visibleIds(index),
          at = ids.indexOf(e.target.closest("[data-tab-id]").dataset.tabId);
        if (e.key === "Delete") {
          e.preventDefault();
          close(ids[at]);
          return;
        }
        const next = {
          ArrowRight: (at + 1) % ids.length,
          ArrowLeft: (at + ids.length - 1) % ids.length,
          Home: 0,
          End: ids.length - 1,
        }[e.key];
        if (next === undefined) return;
        e.preventDefault();
        reveal(ids[next]);
        tabs.get(ids[next]).firstElementChild.focus();
      },
      { signal },
    );
    group.empty.addEventListener(
      "click",
      (e) => {
        const b = e.target.closest("[data-open-tab]");
        if (b) choose(b.dataset.openTab, index);
      },
      { signal },
    );
  }
  for (const event of ["pointerdown", "focusin"])
    surfaces.addEventListener(
      event,
      (e) => {
        const root = e.target.closest("[data-workspace-group]");
        if (root) {
          focus(Number(root.dataset.workspaceGroup));
          records.find((r) => r.element === root)?.onSelect?.();
        }
      },
      { signal },
    );
  picker.addEventListener(
    "click",
    (e) => {
      const b = e.target.closest("[data-open-tab]");
      if (b) choose(b.dataset.openTab);
    },
    { signal },
  );
  document.addEventListener(
    "click",
    (e) => {
      if (e.target.closest("[data-workspace-split]")) split();
      if (e.target.closest("[data-workspace-add]"))
        showPicker(layout.focused, groups[layout.focused].add);
    },
    { signal },
  );
  function ratio(value) {
    const width = layout.groups.length === 2 ? element.clientWidth : 0;
    const minimum =
      layout.groups.length === 2 && width > 0
        ? Math.max(0.2, Math.min(0.5, 320 / Math.max(1, width - 6)))
        : 0.2;
    layout.ratio = Math.max(minimum, Math.min(1 - minimum, value));
    const next = `${layout.ratio * 100}%`;
    if (element.style.getPropertyValue("--workspace-ratio") !== next)
      element.style.setProperty("--workspace-ratio", next);
    handle.setAttribute(
      "aria-valuenow",
      String(Math.round(layout.ratio * 100)),
    );
  }
  handle.addEventListener(
    "pointerdown",
    (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      drag = e.pointerId;
      handle.setPointerCapture(drag);
      element.classList.add("resizing");
    },
    { signal },
  );
  handle.addEventListener(
    "pointermove",
    (e) => {
      if (drag !== e.pointerId) return;
      const r = element.getBoundingClientRect();
      ratio((e.clientX - r.left) / r.width);
    },
    { signal },
  );
  function end() {
    if (drag === undefined) return;
    drag = undefined;
    element.classList.remove("resizing");
    save();
  }
  for (const event of ["pointerup", "pointercancel", "lostpointercapture"])
    handle.addEventListener(event, end, { signal });
  window.addEventListener("blur", end, { signal });
  handle.addEventListener(
    "dblclick",
    () => {
      ratio(0.5);
      save();
    },
    { signal },
  );
  handle.addEventListener(
    "keydown",
    (e) => {
      if (!["ArrowLeft", "ArrowRight", "Home"].includes(e.key)) return;
      e.preventDefault();
      ratio(
        e.key === "Home"
          ? 0.5
          : layout.ratio + (e.key === "ArrowLeft" ? -0.025 : 0.025),
      );
      save();
    },
    { signal },
  );
  const resizeObserver = new ResizeObserver(() => {
    if (layout.groups.length === 2) ratio(layout.ratio);
  });
  resizeObserver.observe(element);
  render();
  return {
    element,
    reveal,
    has: (id) => layout.groups.some((g) => g.ids.includes(id)),
    connect(key, callbacks) {
      const source = { ...callbacks, records: [] };
      sources.set(key, source);
      return {
        update(next) {
          source.records = next;
          render();
        },
        reveal,
        stop() {
          if (sources.get(key) === source) sources.delete(key);
          render();
        },
      };
    },
    stop() {
      pickerMotion.dispose();
      for (const stop of viewMotion.values()) stop();
      viewMotion.clear();
      controller.abort();
      resizeObserver.disconnect();
      element.remove();
    },
  };
}
