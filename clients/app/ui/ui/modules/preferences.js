export function form(root, service, signal) {
  const status = document.createElement("p");
  status.className = "settings-status";
  status.setAttribute("role", "status");
  const controls = [];
  function row(label, hint) {
    const row = document.createElement("label");
    row.className = "settings-row";
    const text = document.createElement("span");
    text.className = "settings-label";
    const title = document.createElement("strong");
    title.textContent = label;
    const detail = document.createElement("span");
    detail.className = "settings-hint";
    detail.textContent = hint;
    text.append(title, detail);
    row.append(text);
    root.insertBefore(row, status);
    return row;
  }
  root.append(status);
  function set(key, value) {
    try {
      service.set(key, value);
      status.textContent = "Сохранено на этом устройстве";
    } catch (error) {
      status.textContent = `Не сохранено: ${error.message}`;
    }
    refresh();
  }
  function refresh() {
    const values = service.read();
    for (const update of controls) update(values);
  }
  function toggle(key, label, hint) {
    const r = row(label, hint),
      input = document.createElement("input");
    input.type = "checkbox";
    input.className = "settings-toggle";
    input.setAttribute("aria-label", label);
    if (key === "animations") input.dataset.animationToggle = "";
    r.append(input);
    controls.push((v) => (input.checked = v[key]));
    input.addEventListener("change", () => set(key, input.checked), { signal });
    return input;
  }
  function range(key, label, hint, min, max, step) {
    const r = row(label, hint),
      wrap = document.createElement("span"),
      input = document.createElement("input"),
      output = document.createElement("output");
    wrap.className = "settings-range";
    input.type = "range";
    Object.assign(input, { min, max, step });
    input.setAttribute("aria-label", label);
    wrap.append(input, output);
    r.append(wrap);
    // The track fills up to the thumb centre (16px thumb).
    const fill = () =>
      input.style.setProperty(
        "--range-fill",
        `calc(8px + (100% - 16px) * ${(input.value - min) / (max - min)})`,
      );
    controls.push((v) => {
      input.value = v[key];
      output.textContent = `${v[key]} px`;
      fill();
    });
    input.addEventListener(
      "input",
      () => {
        fill();
        set(key, Number(input.value));
      },
      { signal },
    );
  }
  function select(key, label, hint, options) {
    const r = row(label, hint),
      input = document.createElement("select");
    input.setAttribute("aria-label", label);
    for (const [value, text] of options) input.add(new Option(text, value));
    r.append(input);
    controls.push((v) => (input.value = v[key]));
    input.addEventListener("change", () => set(key, input.value), { signal });
  }
  const stop = service.subscribe(refresh);
  signal.addEventListener("abort", stop, { once: true });
  return { toggle, range, select, refresh, status };
}
