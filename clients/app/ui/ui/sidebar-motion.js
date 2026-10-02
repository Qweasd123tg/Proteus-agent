import { motionAllowed } from "./motion.js";

// The layout reaches its final width immediately. Only the old visual positions
// travel to the new positions; text and the transcript are never scaled.
export function mountSidebarMotion(sidebar) {
  const app = sidebar.closest(".app-layout");
  if (!app) return () => {};
  const controller = new AbortController(),
    { signal } = controller,
    media = matchMedia("(prefers-reduced-motion: reduce)"),
    animations = new Map();
  const nodes = () => [
    ...app.querySelectorAll(
      ".workspace-board,.topbar-left,.topnav,.sidebar-surface,.sidebar-rail-surface,.settings-link > svg,.sidebar-footer-label,.sidebar-footer-status .connection-badge",
    ),
  ];
  const state = () => ({
    collapsed: app.classList.contains("sidebar-collapsed"),
    settings: app.classList.contains("settings-route"),
    resizing: app.classList.contains("resizing"),
  });
  function appearance(node) {
    const style = getComputedStyle(node),
      transform = new DOMMatrixReadOnly(
        style.transform === "none" ? undefined : style.transform,
      );
    return {
      x: transform.m41,
      y: transform.m42,
      opacity: Number(style.opacity),
    };
  }
  function measure() {
    return new Map(
      nodes().map((node) => {
        const visual = appearance(node),
          rect = node.getBoundingClientRect();
        return [
          node,
          {
            left: rect.left - visual.x,
            top: rect.top - visual.y,
            width: rect.width,
            height: rect.height,
            ...visual,
          },
        ];
      }),
    );
  }
  let previousState = state(),
    previous = measure();
  function cancel() {
    for (const animation of animations.values()) animation.cancel();
    animations.clear();
  }
  function settle() {
    cancel();
    previous = measure();
    previousState = state();
  }
  function changed() {
    const nextState = state(),
      oldState = previousState;
    previousState = nextState;
    if (
      nextState.settings ||
      nextState.resizing ||
      oldState.settings ||
      !motionAllowed()
    ) {
      settle();
      return;
    }
    if (nextState.collapsed === oldState.collapsed) return;
    // An interrupted transition still supplies its currently displayed offset.
    // Its cached layout origin predates the just-applied collapsed class.
    const before = new Map(
      [...previous].map(([node, position]) => {
        const visual = animations.has(node) ? appearance(node) : position;
        return [
          node,
          {
            left: position.left + visual.x,
            top: position.top + visual.y,
            opacity: visual.opacity,
          },
        ];
      }),
    );
    cancel();
    previous = measure();
    for (const [node, next] of previous) {
      const old = before.get(node);
      if (!old || !next.width || !next.height) continue;
      const fading = node.matches(
        ".sidebar-surface,.sidebar-rail-surface,.sidebar-footer-label",
      );
      const from = {
          transform: `translate(${old.left - next.left}px, ${old.top - next.top}px)`,
        },
        to = {
          transform: `translate(${next.x}px, ${next.y}px)`,
        };
      if (fading) {
        from.opacity = old.opacity;
        to.opacity = next.opacity;
        from.visibility = to.visibility = "visible";
      }
      if (
        Math.abs(old.left - next.left - next.x) < 0.5 &&
        Math.abs(old.top - next.top - next.y) < 0.5 &&
        (!fading || old.opacity === next.opacity)
      )
        continue;
      const animation = node.animate([from, to], {
        duration: 260,
        easing: "cubic-bezier(.2,.7,.2,1)",
      });
      animations.set(node, animation);
      const release = () => {
        if (animations.get(node) !== animation) return;
        animations.delete(node);
        if (!animations.size) previous = measure();
      };
      animation.addEventListener("finish", release, { once: true });
      animation.addEventListener("cancel", release, { once: true });
    }
  }
  const observer = new MutationObserver(changed);
  observer.observe(app, { attributes: true, attributeFilter: ["class"] });
  const preferences = new MutationObserver(() => {
    if (!motionAllowed()) settle();
  });
  preferences.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-animations"],
  });
  const resize = new ResizeObserver(() => {
    if (!animations.size) previous = measure();
  });
  resize.observe(sidebar);
  const main = app.querySelector(".workspace-main");
  if (main) resize.observe(main);
  // Model/actions can resize the header without resizing the workspace itself.
  for (const node of nodes()) resize.observe(node);
  window.addEventListener("resize", settle, { signal });
  window.addEventListener(
    "proteus-motion-change",
    () => {
      if (!motionAllowed()) settle();
    },
    { signal },
  );
  media.addEventListener("change", settle, { signal });
  return () => {
    controller.abort();
    observer.disconnect();
    preferences.disconnect();
    resize.disconnect();
    cancel();
  };
}
