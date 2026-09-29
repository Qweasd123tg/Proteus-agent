import { motionAllowed } from "../motion.js";

// Animate only the strip when its membership/order changes. Streaming title
// updates and background registry updates must not restart movement.
export function tabMotion(root, signal) {
  const active = new Map();
  const tabs = () => [...root.querySelectorAll(".workspace-tab")];
  const signature = () =>
    tabs()
      .map(
        (tab) =>
          `${tab.closest("[data-group]").dataset.group}:${tab.dataset.tabId}`,
      )
      .join("|");
  const stop = () => {
    for (const animation of active.values()) animation.cancel();
    active.clear();
  };
  const preference = () => {
    if (!motionAllowed()) stop();
  };
  window.addEventListener("proteus-motion-change", preference);
  signal.addEventListener(
    "abort",
    () => {
      stop();
      window.removeEventListener("proteus-motion-change", preference);
    },
    { once: true },
  );
  return () => {
    const before = new Map(
      tabs().map((tab) => [tab, tab.getBoundingClientRect()]),
    );
    const order = signature();
    return () => {
      if (signature() === order || !motionAllowed() || root.closest("[hidden]"))
        return;
      stop();
      for (const tab of tabs()) {
        const old = before.get(tab),
          next = tab.getBoundingClientRect();
        if (!next.width) continue;
        const x = old ? old.left - next.left : 0;
        if (old && Math.abs(x) < 0.5) continue;
        const animation = tab.animate(
          [
            {
              transform: `translate(${x}px,${old ? 0 : -4}px)`,
              opacity: old ? 1 : 0,
            },
            { transform: "translate(0,0)", opacity: 1 },
          ],
          { duration: 240, easing: "cubic-bezier(.2,.8,.2,1)" },
        );
        active.set(tab, animation);
        const finish = () => {
          if (active.get(tab) === animation) active.delete(tab);
        };
        animation.finished.then(finish, finish);
      }
    };
  };
}
