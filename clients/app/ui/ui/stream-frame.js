export function scheduleStreamFlush(callback) {
  const run = () => callback();
  run.proteusStreamFlush = true;
  requestAnimationFrame(run);
}
