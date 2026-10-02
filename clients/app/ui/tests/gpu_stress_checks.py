"""Sustained painted chat/settings frames and process-local sync_file accounting."""
import os
from pathlib import Path


PROBE = r"""(async () => {
  const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
  const frames = async count => { while (count--) await frame(); };
  const root = document.querySelector('.results-panel');
  let rafFrames = 0, switches = 0, minTop = Infinity, maxTop = 0;
  for (let i = 0; i < 1800; i++) {
    root.scrollTop += i % 240 < 120 ? -24 : 24;
    await frame();
    minTop = Math.min(minTop, root.scrollTop);
    maxTop = Math.max(maxTop, root.scrollTop);
    rafFrames++;
  }
  if (maxTop - minTop < 1000) throw Error('Chat did not move during GPU stress');
  document.querySelector('.settings-link').click();
  await frames(20);
  for (let i = 0; i < 120; i++) {
    const id = ['appearance', 'chat', 'extensions'][i % 3];
    document.querySelector(`[data-settings-section="${id}"]`).click();
    await frames(12);
    const section = document.querySelector(`[data-module-page="${id}"]`);
    if (!section || section.hidden || section.inert || !section.getBoundingClientRect().height)
      throw Error('Selected settings section is not visible: ' + id);
    switches++; rafFrames += 12;
  }
  document.querySelector('.settings-back').click();
  await frames(20);
  return {rafFrames, switches, minTop, maxTop};
})()"""


class SyncFileMonitor:
    def __init__(self):
        self.samples = {}

    def sample(self):
        # Select only descendants of this fixture, never other desktop apps.
        parents, names = {}, {}
        for process in Path('/proc').iterdir():
            if not process.name.isdecimal():
                continue
            try:
                fields = (process / 'stat').read_text().rsplit(')', 1)[1].split()
                parents[int(process.name)] = int(fields[1])
                names[int(process.name)] = (process / 'comm').read_text().strip()
            except (OSError, IndexError, ValueError):
                continue
        for pid, name in names.items():
            if not name.startswith('WebKit'):
                continue
            ancestor, seen = pid, set()
            while ancestor in parents and ancestor not in seen and ancestor != os.getpid():
                seen.add(ancestor)
                ancestor = parents[ancestor]
            if ancestor != os.getpid():
                continue
            try:
                descriptors = list(Path(f'/proc/{pid}/fd').iterdir())
                sync = sum(os.readlink(fd) == 'anon_inode:sync_file' for fd in descriptors)
            except OSError:
                continue
            item = self.samples.setdefault(pid, {'name': name, 'initialSync': sync, 'maxSync': sync, 'lastSync': sync, 'maxFD': len(descriptors), 'samples': 0})
            item.update(maxSync=max(item['maxSync'], sync), lastSync=sync,
                        maxFD=max(item['maxFD'], len(descriptors)), samples=item['samples'] + 1)
        return self.report()

    def report(self):
        return {str(pid): item for pid, item in self.samples.items()}

    def validate(self):
        assert any(item['name'].startswith('WebKitWeb') and item['samples'] >= 10
                   for item in self.samples.values()), 'No sustained WebKit FD samples'
        self.validate_growth()

    def validate_growth(self):
        for item in self.samples.values():
            # Allow a few outstanding buffers/fences, not one leaked FD per frame.
            assert item['maxSync'] <= 128, 'GPU sync_file descriptors already accumulated: ' + str(item)
            assert item['maxSync'] - item['initialSync'] <= 64, 'GPU sync_file descriptors grow: ' + str(item)
