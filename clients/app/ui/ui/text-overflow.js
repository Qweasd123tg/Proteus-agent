import { motionAllowed } from "./motion.js";

// Labels truncated by CSS (`overflow: hidden; white-space: nowrap;
// text-overflow: ellipsis`) fade out instead of showing "…" and slowly scroll
// to reveal the rest while hovered. Rules are discovered in the loaded
// stylesheets, so every new truncating label gets the same behaviour.
// Without scroll-driven animations or with animations off the ellipsis stays.

const supported = CSS.supports("animation-timeline", "scroll(self inline)");
const SPEED = 28;
const MIN_DURATION = 1200;
const RETURN_SPEED = 220;
const RETURN_MIN_DURATION = 320;
const START_DELAY = 600;
const EDGE_PAUSE = 1400;
let selector = "";
let style;
let active = null;
const frames = new WeakMap();

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

// While the label is scrolled by hover, the fade follows the hidden width in
// pixels on every frame: the scroll timeline ramps by percent of the distance,
// so on long labels it lags and the text shows a hard cut before it fades.
function paint(element) {
  const size = parseFloat(getComputedStyle(element).getPropertyValue("--text-fade-size")) || 28;
  const distance = element.scrollWidth - element.clientWidth;
  const ramp = Math.max(1, Math.min(size, distance));
  const left = element.scrollLeft;
  const start = size * Math.min(1, left / ramp);
  const end = size * Math.min(1, Math.max(0, distance - left) / ramp);
  // Important inline values override the running animation without
  // restarting it; WebKit does not revive a removed scroll-driven animation.
  element.style.setProperty(
    "mask-image",
    `linear-gradient(to right, transparent, #000 ${start}px, #000 calc(100% - ${end}px), transparent)`,
    "important",
  );
}

function release(element) {
  cancelAnimationFrame(frames.get(element));
  frames.delete(element);
  element.style.removeProperty("mask-image");
}

function glide(element, to, { speed, minimum, delay = 0 }, done) {
  cancelAnimationFrame(frames.get(element));
  const from = element.scrollLeft;
  const duration = Math.max(minimum, (Math.abs(to - from) / speed) * 1000);
  const startAt = performance.now() + delay;
  let began = null;
  const tick = (now) => {
    if (now >= startAt) {
      began ??= now;
      const progress = Math.min(1, (now - began) / duration);
      element.scrollLeft = from + (to - from) * (0.5 - Math.cos(Math.PI * progress) / 2);
      paint(element);
      if (progress === 1) return done();
    }
    frames.set(element, requestAnimationFrame(tick));
  };
  frames.set(element, requestAnimationFrame(tick));
}

function stop() {
  const element = active;
  if (!element) return;
  active = null;
  if (!element.scrollLeft) return release(element);
  glide(element, 0, { speed: RETURN_SPEED, minimum: RETURN_MIN_DURATION }, () => release(element));
}

function start(element) {
  active = element;
  paint(element);
  const sweep = (delay) => {
    const distance = element.scrollWidth - element.clientWidth;
    const to = element.scrollLeft < distance / 2 ? distance : 0;
    glide(element, to, { speed: SPEED, minimum: MIN_DURATION, delay }, () => {
      if (active === element) sweep(EDGE_PAUSE);
    });
  };
  sweep(element.scrollLeft ? 0 : START_DELAY);
}

if (supported) {
  scan();
  new MutationObserver(scan).observe(document.head, { childList: true });
  document.addEventListener("load", (event) => event.target instanceof HTMLLinkElement && scan(), true);
  document.addEventListener("pointerover", (event) => {
    const element = motionAllowed() ? label(event.target) : null;
    if (element === active) return;
    stop();
    if (element) start(element);
  });
  document.addEventListener("pointerout", (event) => {
    if (active && !active.contains(event.relatedTarget)) stop();
  });
  window.addEventListener("blur", stop);
}
