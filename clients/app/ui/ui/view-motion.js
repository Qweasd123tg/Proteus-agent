import { motionAllowed } from "./motion.js";

// The owner changes hidden/inert immediately. The outgoing, inert view keeps
// only its last painted rectangle until the fade completes; its DOM never moves.
export function watchViewMotion(element, { signal, inPlace = false } = {}) {
  let shown = !element.hidden,
    frame,
    animation,
    geometry,
    stopped = false;
  const properties = ["left", "top", "width", "height", "display"];
  function remember() {
    if (!shown || element.hidden || animation || stopped) return;
    const rect = element.getBoundingClientRect();
    if (rect.width && rect.height)
      geometry = {
        left: `${rect.left}px`,
        top: `${rect.top}px`,
        width: `${rect.width}px`,
        height: `${rect.height}px`,
        display: getComputedStyle(element).display,
      };
  }
  function clearExit() {
    delete element.dataset.viewExit;
    for (const property of properties)
      element.style.removeProperty(`--view-${property}`);
  }
  function settle() {
    const previous = animation;
    animation = undefined;
    previous?.cancel();
    clearExit();
    remember();
  }
  function changed() {
    const next = !element.hidden;
    if (shown === next) return;
    const interrupted = !!animation;
    const opacity = interrupted
      ? Number(getComputedStyle(element).opacity)
      : next
        ? 0
        : 1;
    shown = next;
    element.inert = !shown;
    settle();
    // A parent screen already hides nested sections; do not paint through it.
    if (!motionAllowed() || element.parentElement?.closest("[hidden]")) return;
    if (!shown && inPlace) {
      if (!geometry) return;
      // Workspace surfaces already overlap without participating in layout.
      // Keep that geometry: fixing a long transcript forces another full layout.
      element.style.setProperty("--view-display", geometry.display);
      element.dataset.viewExit = "in-place";
    } else if (!shown) {
      if (!geometry) return;
      // An ancestor's own movement already animates this region. A second
      // fixed exit would use a moving coordinate system and visibly drift.
      for (
        let parent = element.parentElement;
        parent;
        parent = parent.parentElement
      ) {
        const style = getComputedStyle(parent);
        if (style.transform !== "none" || style.perspective !== "none") return;
      }
      for (const property of properties)
        element.style.setProperty(`--view-${property}`, geometry[property]);
      element.dataset.viewExit = "";
      // Measure the actual fixed coordinate system: containment differs across
      // engines. No transformed ancestor remains, so a translation is enough.
      const painted = element.getBoundingClientRect();
      for (const property of ["left", "top"]) {
        const origin = parseFloat(geometry[property]);
        element.style.setProperty(
          `--view-${property}`,
          `${origin + origin - painted[property]}px`,
        );
      }
    }
    const current = element.animate([{ opacity }, { opacity: shown ? 1 : 0 }], {
      duration: shown ? 240 : 140,
      easing: "cubic-bezier(.2,.8,.2,1)",
    });
    animation = current;
    const finish = () => {
      if (animation === current) settle();
    };
    current.finished.then(finish, finish);
  }
  const observer = new MutationObserver(changed);
  observer.observe(element, { attributes: true, attributeFilter: ["hidden"] });
  const resize = new ResizeObserver(remember);
  resize.observe(element);
  const refresh = () => {
    if (!shown || element.hidden || animation || stopped || frame) return;
    frame = requestAnimationFrame(() => {
      frame = undefined;
      remember();
    });
  };
  // Scrolling can move a settings section without changing its size.
  const scrolled = (event) => {
    const target = event.target;
    // An internal transcript/tree scroll does not move the view's own box.
    // Only scrolling an ancestor changes the exit geometry we remember.
    if (target === document || target === window
      || (target !== element && target?.contains?.(element))) refresh();
  };
  window.addEventListener("scroll", scrolled, true);
  window.addEventListener("resize", refresh);
  const preference = () => {
    if (!motionAllowed()) settle();
  };
  window.addEventListener("proteus-motion-change", preference);
  remember();
  const stop = () => {
    if (stopped) return;
    stopped = true;
    observer.disconnect();
    resize.disconnect();
    cancelAnimationFrame(frame);
    window.removeEventListener("scroll", scrolled, true);
    window.removeEventListener("resize", refresh);
    window.removeEventListener("proteus-motion-change", preference);
    signal?.removeEventListener("abort", stop);
    settle();
  };
  signal?.addEventListener("abort", stop, { once: true });
  return stop;
}
