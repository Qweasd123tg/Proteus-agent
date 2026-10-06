import { motionAllowed } from "./motion.js";

// Labels truncated by CSS (`overflow: hidden; white-space: nowrap;
// text-overflow: ellipsis`) fade out instead of showing "…" and slowly scroll
// to reveal the rest while hovered. Rules are discovered in the loaded
// stylesheets, so every new truncating label gets the same behaviour.
// Without scroll-driven animations or with animations off the ellipsis stays.

const supported = CSS.supports("animation-timeline", "scroll(self inline)");
const SPEED = 45;
const START_DELAY = 450;
const EDGE_PAUSE = 1100;
let selector = "";
let style;
let active = null;

function truncatingSelectors(rules, into) {
  for (const rule of rules) {
    if (rule.cssRules && !(rule instanceof CSSStyleRule)) {
      truncatingSelectors(rule.cssRules, into);
      continue;
    }
    if (!(rule instanceof CSSStyleRule) || rule.style.textOverflow !== "ellipsis") continue;
    // Bare element rules (the base `button`) and animated labels keep their own behaviour.
    if (rule.style.animationName && rule.style.animationName !== "none") continue;
    for (const part of rule.selectorText.split(",")) {
      const value = part.trim();
      if (value && !/^[a-z]+$/i.test(value)) into.add(value);
    }
  }
  return into;
}

function scan() {
  const selectors = new Set();
  for (const sheet of document.styleSheets) {
    if (sheet.ownerNode === style) continue;
    let rules;
    try {
      rules = sheet.cssRules;
    } catch {
      continue;
    }
    truncatingSelectors(rules, selectors);
  }
  const list = [...selectors];
  const next = list.join(",");
  if (next === selector) return;
  selector = next;
  if (!list.length) return;
  style ??= document.head.appendChild(document.createElement("style"));
  style.dataset.textOverflow = "";
  const scoped = list.map((value) => `html:not([data-animations=off]) ${value}`).join(",\n");
  style.textContent = `${scoped} {
  text-overflow: clip;
  animation-name: text-fade;
  animation-duration: auto;
  animation-timing-function: linear;
  animation-fill-mode: both;
  animation-timeline: scroll(self inline);
}`;
}

function label(target) {
  if (!selector || !(target instanceof Element)) return null;
  const element = target.closest(selector);
  return element && element.scrollWidth - element.clientWidth > 1 ? element : null;
}

function stop() {
  if (!active) return;
  const { element, frame } = active;
  active = null;
  cancelAnimationFrame(frame);
  if (element.scrollLeft) element.scrollTo({ left: 0, behavior: "smooth" });
}

function start(element) {
  const distance = element.scrollWidth - element.clientWidth;
  const duration = Math.max(600, (distance / SPEED) * 1000);
  let forward = true;
  let began = null;
  let resume = performance.now() + START_DELAY;
  const tick = (now) => {
    if (active?.element !== element) return;
    if (now >= resume) {
      began ??= now;
      const progress = Math.min(1, (now - began) / duration);
      const eased = progress < 0.5 ? 2 * progress * progress : 1 - (-2 * progress + 2) ** 2 / 2;
      element.scrollLeft = (forward ? eased : 1 - eased) * distance;
      if (progress === 1) {
        forward = !forward;
        began = null;
        resume = now + EDGE_PAUSE;
      }
    }
    active.frame = requestAnimationFrame(tick);
  };
  active = { element, frame: requestAnimationFrame(tick) };
}

if (supported) {
  scan();
  new MutationObserver(scan).observe(document.head, { childList: true });
  document.addEventListener("load", (event) => event.target instanceof HTMLLinkElement && scan(), true);
  document.addEventListener("pointerover", (event) => {
    const element = motionAllowed() ? label(event.target) : null;
    if (element === active?.element) return;
    stop();
    if (element) start(element);
  });
  document.addEventListener("pointerout", (event) => {
    if (active && !active.element.contains(event.relatedTarget)) stop();
  });
  window.addEventListener("blur", stop);
}
