// Gentle Shell's scene kit: what a theme's scene draws with. It runs in a
// sandboxed QuickJS with no files, network or timers; the app calls
// __frame(width, height, t) about 30 times a second and paints the commands
// it leaves in __buf.
//
// The ASCII helpers and the cell/sprite engine are adapted from
// claude-desktop-themes by 0toshigami (MIT, see scenes/LICENSE), so the
// scenes written for that mod run here unchanged.

// ── Output: 10 numbers per command (op, then up to 9 arguments) ────────────
let __buf = new Float64Array(10 * 8192), __n = 0;
const __push = (op, a, b, c, d, e, f, g, h, k = 0) => {
  if ((__n + 1) * 10 > __buf.length) {
    const grown = new Float64Array(__buf.length * 2);
    grown.set(__buf);
    __buf = grown;
  }
  const i = __n++ * 10;
  __buf[i] = op; __buf[i + 1] = a; __buf[i + 2] = b; __buf[i + 3] = c; __buf[i + 4] = d;
  __buf[i + 5] = e; __buf[i + 6] = f; __buf[i + 7] = g; __buf[i + 8] = h; __buf[i + 9] = k;
};

// A CSS-ish color ('#rgb', '#rrggbb', '#rrggbbaa', 'rgb(…)', 'rgba(…)') as
// [0xRRGGBB, alpha]; anything else is white.
const __colors = new Map();
const __color = c => {
  if (typeof c === 'number') return [c & 0xffffff, 1];
  let hit = __colors.get(c);
  if (hit) return hit;
  let rgb = 0xffffff, a = 1;
  const s = String(c).trim();
  if (s[0] === '#') {
    let h = s.slice(1);
    if (h.length === 3 || h.length === 4) h = [...h].map(x => x + x).join('');
    rgb = parseInt(h.slice(0, 6), 16) || 0;
    if (h.length === 8) a = parseInt(h.slice(6, 8), 16) / 255;
  } else {
    const m = /rgba?\(([^)]*)\)/i.exec(s);
    if (m) {
      const p = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
      rgb = ((p[0] & 255) << 16) | ((p[1] & 255) << 8) | (p[2] & 255);
      if (p.length > 3) a = p[3];
    }
  }
  hit = [rgb, a];
  if (__colors.size < 4096) __colors.set(c, hit);
  return hit;
};
const __clamp = a => (a > 1 ? 1 : a > 0 ? a : 0);

// ── The drawing API a scene's draw(g, t) gets ──────────────────────────────
const __g = {
  width: 0, height: 0, t: 0,
  /** Colors of the theme: bg, text, accent, muted, success, danger, warning, info. */
  theme: {},
  rect(x, y, w, h, color, alpha = 1, radius = 0) {
    const [c, a] = __color(color);
    __push(1, x, y, w, h, radius, c, __clamp(alpha * a), 0);
  },
  circle(x, y, r, color, alpha = 1) {
    const [c, a] = __color(color);
    __push(2, x, y, r, r, c, __clamp(alpha * a), 0, 0);
  },
  ellipse(x, y, rx, ry, color, alpha = 1) {
    const [c, a] = __color(color);
    __push(2, x, y, rx, ry, c, __clamp(alpha * a), 0, 0);
  },
  /** Text centered on (x, y), one monospace cell per character. */
  text(x, y, text, color, alpha = 1, size = 13, bold = false) {
    const [c, a] = __color(color), s = String(text), step = size * 0.6;
    let cx = x - (step * (s.length - 1)) / 2;
    for (const ch of s) {
      if (ch !== ' ') __push(3, cx, y, size, ch.codePointAt(0), c, __clamp(alpha * a), bold ? 1 : 0, 0);
      cx += step;
    }
  },
  /** A soft radial glow fading out at radius r. */
  glow(x, y, r, color, alpha = 1) {
    const [c, a] = __color(color);
    __push(4, x, y, r, c, __clamp(alpha * a), 0, 0, 0);
  },
  /** A linear gradient filling the rect; angle in degrees, 180 = top to bottom. */
  gradient(x, y, w, h, angle, from, to, alpha = 1) {
    const [c1, a1] = __color(from), [c2, a2] = __color(to);
    __push(5, x, y, w, h, angle, c1, c2, __clamp(alpha * a1), __clamp(alpha * a2));
  },
  line(x1, y1, x2, y2, color, alpha = 1, width = 1) {
    const [c, a] = __color(color);
    __push(6, x1, y1, x2, y2, width, c, __clamp(alpha * a), 0);
  },
};

// ── ASCII scene helpers (claude-desktop-themes core.js, MIT) ───────────────
let CELL = 22, SIDE = 0;
const root = { dataset: {} };
const GLYPHS = 'St%;:X', LOOSE = ".:',o";
const rgb = c => { const [v] = __color(c); return [(v >> 16) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255]; };
const hex = c => '#' + c.map(v => Math.round(v * 255).toString(16).padStart(2, '0')).join('');
const mix = (a, b, k) => a.map((v, i) => v + (b[i] - v) * k);
const hsl = ([r, gg, b]) => {
  const hi = Math.max(r, gg, b), lo = Math.min(r, gg, b), l = (hi + lo) / 2, d = hi - lo;
  const s = d ? d / (1 - Math.abs(2 * l - 1)) : 0;
  const h = !d ? 0 : hi === r ? ((gg - b) / d + 6) % 6 : hi === gg ? (b - r) / d + 2 : (r - gg) / d + 4;
  return `${Math.round(h * 60)} ${(s * 100).toFixed(1)}% ${(l * 100).toFixed(1)}%`;
};
const hash = (a, b, c = 0) => {
  let h = Math.imul(a, 374761393) ^ Math.imul(b, 668265263) ^ Math.imul(c, 1442695041);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
};
const glyph = (set, x, y, t) => set[Math.floor(hash(x, y, Math.floor(t * 1.2 + hash(y, x) * 7)) * set.length)];
const ell = (x, y, cx, cy, rx, ry) => ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2;
const drift = (cols, f, t, v, w) => { const span = cols + w * 2; return ((f * span + t * v) % span) - w; };
const near = (x, y, fg, a = .3, odds = .5) => (hash(x, y, 3) < odds ? { fg, a } : undefined);
const twinkle = (x, y, t, fg, odds) => {
  const s = hash(x, y, 9);
  if (s < odds) return { ch: s < odds / 3 ? '+' : '.', fg, a: .2 + .5 * (.5 + .5 * Math.sin(t * (1 + s * 200) + s * 1000)) };
};
const rising = (x, y, t, v, odds) => hash(x, Math.floor(y + t * v), 11) < odds;
const sketch = (w, h, parts) => {
  const grid = Array.from({ length: h }, () => Array(w).fill('.'));
  for (const [r, c, s] of parts) [...s].forEach((ch, i) => { if (ch !== ' ' && grid[r]?.[c + i]) grid[r][c + i] = ch; });
  return grid.map(r => r.join(''));
};
const shape = (w, h, at) => Array.from({ length: h }, (_, y) => Array.from({ length: w }, (_, x) => at(x, y)).join(''));
const flip = art => art.map(r => [...r].reverse().join(''));
const frame = (frames, t, rate, phase = 0) => frames[Math.floor(t * rate + phase) % frames.length];
const shade = (c, k) => hex(mix(rgb(c), k > 0 ? [1, 1, 1] : [0, 0, 0], Math.abs(k)));
const tint = (bg, fg, alphas) => Object.fromEntries(alphas.map((a, i) => [String(i + 1), { bg, fg, a }]));
const halos = new WeakMap();
const haloOf = art => {
  if (!halos.has(art)) {
    const on = (x, y) => (art[y]?.[x] ?? '.') !== '.', out = [];
    for (let y = -1; y <= art.length; y++)
      for (let x = -1; x <= art[0].length; x++)
        if (!on(x, y) && [-1, 0, 1].some(i => [-1, 0, 1].some(j => on(x + i, y + j))) && hash(x + 50, y + 50, 5) < .45) out.push([x, y]);
    halos.set(art, out);
  }
  return halos.get(art);
};

// A scene file adds itself: SCENES['<id>'] = { draw?, cell?, sprites?, size? }.
const SCENES = {};

// ── The cell/sprite engine (claude-desktop-themes engine.js, MIT) ──────────
const __put = (px, py, c, sx, sy, t) => {
  const a0 = c.a ?? 1, a = a0 > 1 ? 1 : a0 > 0 ? a0 : 0;
  if (a < .01) return;
  // (inlined: this runs for every cell of every frame)
  if (c.bg) {
    const bg = __color(c.bg);
    __push(1, px + 2, py + 2, CELL - 4, CELL - 4, 3, bg[0], a * bg[1], 0);
  }
  const ch = c.ch ?? glyph(c.bg ? GLYPHS : LOOSE, sx, sy, t);
  if (ch && c.fg) {
    const fg = __color(c.fg);
    __push(3, px + CELL / 2, py + CELL / 2 + 1, __glyphSize, ch.codePointAt(0), fg[0], a * fg[1], 1, 0);
  }
};
let __glyphSize = 13;

const __ascii = (scene, W, H, t) => {
  const cols = Math.ceil(W / CELL), rows = Math.ceil(H / CELL);
  for (let x = 0; x <= cols; x++) __push(1, x * CELL, 0, 1, H, 0, 0xa0b4ff, .05, 0);
  for (let y = 0; y <= rows; y++) __push(1, 0, y * CELL, W, 1, 0, 0xa0b4ff, .05, 0);
  if (scene.cell)
    for (let y = 0; y < rows; y++)
      for (let x = 0; x < cols; x++) {
        const c = scene.cell(x, y, cols, rows, t);
        if (c) __put(x * CELL, y * CELL, c, x, y, t);
      }
  for (const s of scene.sprites?.(cols, rows, t) ?? []) {
    const ox = s.x * CELL, oy = s.y * CELL;
    const m = s.a ?? 1, k = (s.key ?? 0) * 997;
    const bw = s.art ? s.art[0].length : s.w, bh = s.art ? s.art.length : s.h;
    if (ox > W + CELL * 4 || oy > H + CELL * 4 || ox + (bw + 4) * CELL < 0 || oy + (bh + 4) * CELL < 0) continue;
    if (s.glow) {
      const gl = s.glow;
      __g.glow(ox + bw / 2 * CELL, oy + bh / 2 * CELL, gl.r * CELL, gl.color, Math.min(1, gl.a));
    }
    const cells = [];
    if (s.art) s.art.forEach((row, gy) => { for (let gx = 0; gx < row.length; gx++) if (s.pal[row[gx]]) cells.push([gx, gy, s.pal[row[gx]], gx + k, gy]); });
    else
      for (let gy = 0; gy < s.h; gy++)
        for (let gx = 0; gx < s.w; gx++) {
          const f = s.field(gx, gy);
          if (f) cells.push([gx, gy, f.c, (f.sx ?? gx) + k, f.sy ?? gy]);
        }
    if (s.shadow)
      for (const [gx, gy, c] of cells)
        if (c.bg) __put(ox + (gx + .8) * CELL, oy + (gy + .8) * CELL, { bg: s.shadow, fg: s.shadow, ch: '', a: .4 * m }, 0, 0, t);
    if (s.halo && s.art)
      for (const [gx, gy] of haloOf(s.art))
        __put(ox + gx * CELL, oy + gy * CELL, { fg: s.halo, a: .3 * m }, gx + k, gy, t);
    for (const [gx, gy, c, sx, sy] of cells) __put(ox + gx * CELL, oy + gy * CELL, m === 1 ? c : { ...c, a: (c.a ?? 1) * m }, sx, sy, t);
  }
};

// ── Entry points the app calls ─────────────────────────────────────────────
let __scene;
const __pick = () => {
  const keys = Object.keys(SCENES);
  __scene = keys.length ? SCENES[keys[keys.length - 1]] : undefined;
  return !!__scene;
};
const __frame = (W, H, t, cell) => {
  __n = 0;
  __g.width = W; __g.height = H; __g.t = t;
  const most = typeof __scene.size === 'number' ? __scene.size : 22;
  CELL = Math.max(8, Math.min(cell, most));
  __glyphSize = Math.round(CELL * 13 / 22);
  if (__scene.draw) __scene.draw(__g, t);
  if (__scene.cell || __scene.sprites) __ascii(__scene, W, H, t);
  return __n;
};

// (top-level const/let aren't properties of globalThis: this is how the app reaches them)
globalThis.__api = {
  pick: __pick,
  frame: __frame,
  out: () => __buf,
  setTheme: theme => { __g.theme = theme; },
  setState: (state, event) => { root.dataset.themeState = state || undefined; root.dataset.themeEvent = event || undefined; },
};
