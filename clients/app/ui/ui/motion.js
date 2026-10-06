const key = "proteus.animations";
const reduced = matchMedia("(prefers-reduced-motion: reduce)");
export const motionAllowed = () =>
  document.documentElement.dataset.animations !== "off" && !reduced.matches;
export function applyMotion() {
  let enabled = true;
  try {
    enabled = localStorage.getItem(key) !== "false";
  } catch {}
  document.documentElement.dataset.animations = enabled ? "on" : "off";
  // The label fade is driven by scroll position, not time.
  if (!motionAllowed())
    for (const animation of document.getAnimations())
      if (animation.animationName !== "text-fade") animation.cancel();
  window.dispatchEvent(new Event("proteus-motion-change"));
}
applyMotion();
window.addEventListener("storage", (event) => {
  if (event.key === key) applyMotion();
});
reduced.addEventListener("change", applyMotion);
document.addEventListener("change", (event) => {
  if (event.target.matches("[data-animation-toggle]"))
    queueMicrotask(applyMotion);
});
