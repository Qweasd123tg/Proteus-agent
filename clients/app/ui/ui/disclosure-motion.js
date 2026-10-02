import { motionAllowed } from "./motion.js";

// Explicitly mounted on small settings disclosures, never on the transcript.
export function mountDisclosureMotion(details, content, signal) {
  const summary = details.querySelector("summary");
  let expanded = details.open,
    animation;
  const height = details.style.height,
    overflow = details.style.overflow;
  function semantics() {
    summary.setAttribute("aria-expanded", String(expanded));
    content.inert = !expanded;
  }
  function settle() {
    const previous = animation;
    animation = undefined;
    previous?.cancel();
    details.open = expanded;
    details.style.height = height;
    details.style.overflow = overflow;
    semantics();
  }
  function toggle(event) {
    if (event.button !== 0) return;
    event.preventDefault();
    const before = details.getBoundingClientRect().height;
    expanded = !expanded;
    settle();
    if (!motionAllowed()) return;
    details.open = true;
    const style = getComputedStyle(details);
    const closed =
      summary.getBoundingClientRect().height +
      [
        "paddingTop",
        "paddingBottom",
        "borderTopWidth",
        "borderBottomWidth",
      ].reduce((sum, name) => sum + (parseFloat(style[name]) || 0), 0);
    const after = expanded ? details.getBoundingClientRect().height : closed;
    details.style.overflow = "hidden";
    const current = details.animate(
      [{ height: `${before}px` }, { height: `${after}px` }],
      {
        duration: expanded ? 240 : 180,
        easing: "cubic-bezier(.2,.8,.2,1)",
      },
    );
    animation = current;
    const finish = () => {
      if (animation === current) settle();
    };
    current.finished.then(finish, finish);
  }
  summary.addEventListener("click", toggle, { signal });
  const observer = new MutationObserver(() => {
    // Programmatic closure (leaving Settings or restoring modules) wins.
    if (!details.open && animation) {
      expanded = false;
      settle();
    } else if (!animation) {
      expanded = details.open;
      semantics();
    }
  });
  observer.observe(details, { attributes: true, attributeFilter: ["open"] });
  const preference = () => {
    if (!motionAllowed()) settle();
  };
  window.addEventListener("proteus-motion-change", preference);
  semantics();
  signal.addEventListener(
    "abort",
    () => {
      observer.disconnect();
      window.removeEventListener("proteus-motion-change", preference);
      settle();
    },
    { once: true },
  );
}
