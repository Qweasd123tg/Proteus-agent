import { icon } from "../../extensions/icons.js";
export function menu(root, kind, label, glyph, signal) {
  const details = document.createElement("details");
  details.className = `composer-menu composer-${kind}-menu`;
  const summary = document.createElement("summary");
  summary.className = "composer-menu-trigger";
  summary.setAttribute("aria-label", label);
  if (glyph) summary.append(icon(glyph));
  const name = document.createElement("span");
  name.className = "composer-menu-model";
  summary.append(name);
  const meta = document.createElement("span");
  meta.className = "composer-menu-meta";
  summary.append(meta, icon("chevron-down"));
  const panel = document.createElement("div");
  panel.className = "composer-menu-panel";
  panel.setAttribute("popover", "auto");
  details.append(summary, panel);
  root.append(details);
  function position() {
    const r = summary.getBoundingClientRect(),
      size = panel.getBoundingClientRect();
    panel.style.left = `${Math.max(8, Math.min(r.left, innerWidth - size.width - 8))}px`;
    panel.style.top = `${Math.max(8, Math.min(r.bottom + size.height + 8 < innerHeight ? r.bottom + 6 : r.top - size.height - 6, innerHeight - size.height - 8))}px`;
  }
  details.addEventListener(
    "toggle",
    () => {
      if (details.open) {
        for (const other of document.querySelectorAll(".composer-menu[open]"))
          if (other !== details) other.open = false;
        panel.showPopover();
        position();
      } else if (panel.matches(":popover-open")) panel.hidePopover();
    },
    { signal },
  );
  panel.addEventListener(
    "toggle",
    () => {
      if (!panel.matches(":popover-open")) details.open = false;
    },
    { signal },
  );
  window.addEventListener(
    "resize",
    () => {
      details.open = false;
    },
    { signal },
  );
  signal.addEventListener(
    "abort",
    () => {
      if (panel.matches(":popover-open")) panel.hidePopover();
    },
    { once: true },
  );
  return { details, summary, name, meta, panel };
}
export function section(panel, title) {
  const node = document.createElement("section");
  node.className = "composer-menu-section";
  const label = document.createElement("span");
  label.className = "composer-menu-label";
  label.textContent = title;
  const options = document.createElement("div");
  options.className = "composer-menu-options stacked";
  node.append(label, options);
  panel.append(node);
  return options;
}
export function option(root, label, active, run, description = "") {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "menu-option menu-option-row";
  button.classList.toggle("active", active);
  const text = document.createElement("span");
  text.className = "menu-option-text";
  const title = document.createElement("span");
  title.className = "menu-option-title";
  title.textContent = label;
  text.append(title);
  if (description) {
    const hint = document.createElement("span");
    hint.className = "menu-option-desc";
    hint.textContent = description;
    text.append(hint);
  }
  const check = document.createElement("span");
  check.className = "menu-option-check";
  check.setAttribute("aria-hidden", "true");
  check.textContent = "✓";
  button.append(text, check);
  button.disabled = !run;
  if (run) button.addEventListener("click", run);
  root.append(button);
  return button;
}
