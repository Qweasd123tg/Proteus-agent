// A closed popover leaves the top layer in current Firefox/WebKit. Containment
// would change its fixed-position origin, so only its inert pixels fade on body.
// Menus need their actual per-node paint and layout, including Shadow DOM theme
// and nth-child rules. Serializing every browser property also copies hundreds
// of irrelevant declarations and stalls native WebKit before native dismissal.
const visualProperties =
  `display position inset z-index box-sizing width height min-width min-height max-width max-height
  margin padding border-width border-style border-color border-radius outline outline-offset
  background box-shadow color opacity visibility overflow-x overflow-y
  font letter-spacing
  text-align text-decoration text-shadow text-transform text-indent text-overflow white-space word-break overflow-wrap
  flex align-self justify-self order grid-column grid-row
  transform transform-origin translate rotate scale filter clip-path
  appearance list-style direction writing-mode vertical-align`.split(/\s+/);
const containerProperties =
  "align-items align-content justify-content justify-items gap".split(" ");
const flexProperties = "flex-direction flex-wrap".split(" ");
const gridProperties =
  "grid-template-columns grid-template-rows grid-auto-flow grid-auto-columns grid-auto-rows".split(
    " ",
  );
const svgProperties =
  "fill stroke stroke-width stroke-linecap stroke-linejoin".split(" ");
const fontProperties =
  "font-family font-size font-weight font-style font-stretch font-variant line-height".split(
    " ",
  );

export function needsExitSnapshot(element) {
  if (CSS.supports("overlay", "auto")) return false;
  for (
    let parent = element.parentElement || element.getRootNode().host;
    parent;
    parent = parent.parentElement || parent.getRootNode().host
  ) {
    const style = getComputedStyle(parent);
    if (
      /(layout|paint|strict|content)/.test(style.contain) ||
      style.containerType !== "normal" ||
      [
        "transform",
        "translate",
        "rotate",
        "scale",
        "filter",
        "perspective",
      ].some((key) => style[key] && style[key] !== "none")
    )
      return true;
  }
  return false;
}

export function exitSnapshot(source) {
  // This visual path is for menus, never live embedded views or media.
  if (
    source.querySelector(
      'iframe,object,embed,video,audio,canvas,[role="tabpanel"]',
    )
  )
    return null;
  const rect = source.getBoundingClientRect(),
    style = getComputedStyle(source);
  const opacity = style.opacity;
  const rawDuration = style.getPropertyValue("--motion-exit").trim() || "140ms";
  const duration =
    parseFloat(rawDuration) * (rawDuration.endsWith("ms") ? 1 : 1000);
  const easing = style.getPropertyValue("--motion-ease").trim() || "ease";
  const copy = source.cloneNode(true),
    originals = [source, ...source.querySelectorAll("*")];
  const copies = [copy, ...copy.querySelectorAll("*")];
  // Read before attaching/writing scroll offsets: alternating live writes and
  // source reads would repeatedly force layout for the entire cloned menu.
  const scroll = originals.map((node) => [node.scrollLeft, node.scrollTop]);
  copies.forEach((node, index) => {
    const computed = getComputedStyle(originals[index]);
    const properties = [...visualProperties];
    if (/flex|grid/.test(computed.display))
      properties.push(...containerProperties);
    if (computed.display.includes("flex")) properties.push(...flexProperties);
    if (computed.display.includes("grid")) properties.push(...gridProperties);
    if (node instanceof SVGElement) properties.push(...svgProperties);
    if (!computed.font) properties.push(...fontProperties);
    if (!index)
      properties.push(...[...computed].filter((key) => key.startsWith("--")));
    node.style.cssText = properties
      .map((key) => `${key}:${computed.getPropertyValue(key)};`)
      .join("");
    for (const attribute of [...node.attributes]) {
      const visualState =
        /^aria-(selected|checked|disabled|pressed|expanded)$/.test(
          attribute.name,
        );
      if (
        [
          "id",
          "role",
          "name",
          "for",
          "tabindex",
          "popover",
          "autofocus",
        ].includes(attribute.name) ||
        /^(data-|on)/.test(attribute.name) ||
        (attribute.name.startsWith("aria-") && !visualState)
      )
        node.removeAttribute(attribute.name);
    }
    node.style.setProperty("transition", "none", "important");
    node.style.setProperty("animation", "none", "important");
  });
  copy.className = "ui-popover-exit";
  copy.inert = true;
  copy.setAttribute("aria-hidden", "true");
  Object.assign(copy.style, {
    position: "fixed",
    inset: "auto",
    left: `${rect.left}px`,
    top: `${rect.top}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
    minWidth: "0",
    minHeight: "0",
    maxWidth: "none",
    maxHeight: "none",
    margin: "0",
    boxSizing: "border-box",
    transform: "none",
    translate: "none",
    rotate: "none",
    scale: "none",
    opacity,
    pointerEvents: "none",
    zIndex: "2147483000",
  });
  document.body.append(copy);
  copies.forEach((node, index) => {
    if (scroll[index][1]) node.scrollTop = scroll[index][1];
    if (scroll[index][0]) node.scrollLeft = scroll[index][0];
  });
  const animation = copy.animate(
    [
      { opacity },
      { opacity: 0 },
    ],
    { duration, easing, fill: "forwards" },
  );
  return {
    finished: animation.finished.catch(() => {}),
    current() {
      const current = getComputedStyle(copy);
      return {
        opacity: current.opacity,
      };
    },
    stop() {
      animation.cancel();
      copy.remove();
    },
  };
}
