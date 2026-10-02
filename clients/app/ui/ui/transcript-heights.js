// Prefix sums keep viewport lookup and individual resize updates O(log n).
export class TranscriptHeights {
  constructor(rows = []) { this.reset(rows); }
  reset(rows) {
    const previous = this.measured || new Map();
    this.measured = previous;
    this.rows = rows;
    this.positions = new Map(rows.map((row, i) => [String(row.id), i]));
    this.values = rows.map(row => previous.get(String(row.id)) ?? row.height);
    this.tree = Array(rows.length + 1).fill(0);
    this.values.forEach((height, i) => this.add(i, height));
    for (const id of previous.keys()) if (!this.positions.has(id)) previous.delete(id);
  }
  add(index, delta) {
    for (let i = index + 1; i < this.tree.length; i += i & -i) this.tree[i] += delta;
  }
  set(id, height) {
    const index = this.positions.get(String(id));
    if (index === undefined || height <= 0) return false;
    this.measured.set(String(id), height);
    const delta = height - this.values[index];
    // Fractional row heights accumulate above the reading anchor. Keep the
    // prefix index and measured cache equal, including changes below 1px.
    if (delta === 0) return false;
    this.values[index] = height;
    this.add(index, delta);
    return true;
  }
  prefix(end) {
    let height = 0;
    for (let i = end; i > 0; i -= i & -i) height += this.tree[i];
    return height;
  }
  get total() { return this.prefix(this.rows.length); }
  at(offset) {
    let index = 0, sum = 0;
    let bit = 1;
    while (bit * 2 < this.tree.length) bit *= 2;
    for (; bit; bit >>= 1) {
      const next = index + bit;
      if (next < this.tree.length && sum + this.tree[next] <= offset) {
        index = next; sum += this.tree[next];
      }
    }
    return Math.min(index, Math.max(0, this.rows.length - 1));
  }
}
