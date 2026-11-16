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

// ── Points of paths and polygons ───────────────────────────────────────────
// x, y pairs; each subpath ends with a NaN pair whose second number is 1 when
// the subpath is closed and 0 when it's open. A path command (op 7 or 8)
// names its first pair and how many pairs it spans.
let __pts = new Float64Array(2 * 16384), __np = 0;
const __pt = (x, y) => {
  if (__np + 2 > __pts.length) {
    const grown = new Float64Array(__pts.length * 2);
    grown.set(__pts);
    __pts = grown;
  }
  __pts[__np++] = x; __pts[__np++] = y;
};

// ── The current transform: x' = a·x + c·y + e, y' = b·x + d·y + f ──────────
let __a = 1, __b = 0, __c = 0, __d = 1, __e = 0, __f = 0, __plain = true;
const __saved = [];
const __settle = () => { __plain = __a === 1 && __b === 0 && __c === 0 && __d === 1 && __e === 0 && __f === 0; };
const __tx = (x, y) => __a * x + __c * y + __e;
const __ty = (x, y) => __b * x + __d * y + __f;
/** How much lengths grow under the transform (the mean of both axes). */
const __grow = () => Math.sqrt(Math.abs(__a * __d - __b * __c));
const __turned = () => __b !== 0 || __c !== 0;

// Emit a closed outline of n points from `at(i)` (local coordinates), transformed.
const __outline = (n, at) => {
  for (let i = 0; i < n; i++) { const [x, y] = at(i); __pt(__tx(x, y), __ty(x, y)); }
  __pt(NaN, 1);
};
const __ellipsePoints = (cx, cy, rx, ry) => {
  const n = Math.max(12, Math.min(64, Math.ceil(Math.sqrt(Math.max(rx, ry) * __grow()) * 4)));
  return [n, i => { const k = (i / n) * Math.PI * 2; return [cx + Math.cos(k) * rx, cy + Math.sin(k) * ry]; }];
};
const __rectPoints = (x, y, w, h, r) => {
  r = Math.max(0, Math.min(r, w / 2, h / 2));
  if (r === 0) {
    const corners = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    return [4, i => corners[i]];
  }
  const centers = [[x + w - r, y + r], [x + w - r, y + h - r], [x + r, y + h - r], [x + r, y + r]];
  const steps = 5;
  return [4 * (steps + 1), i => {
    const corner = Math.floor(i / (steps + 1)), s = i % (steps + 1);
    const k = -Math.PI / 2 + (corner + s / steps) * (Math.PI / 2);
    return [centers[corner][0] + Math.cos(k) * r, centers[corner][1] + Math.sin(k) * r];
  }];
};
// A filled outline in the current transform, as one path command.
const __fillOutline = ([n, at], c, alpha) => {
  const start = __np / 2;
  __outline(n, at);
  __push(7, start, __np / 2 - start, 0, c, alpha, 0, 0, 0);
};

// ── The drawing API a scene's draw(g, t) gets ──────────────────────────────
const __g = {
  width: 0, height: 0, t: 0,
  /** Colors of the theme: bg, text, accent, muted, success, danger, warning, info. */
  theme: {},
  /** Where the theme's image is this frame ({ x, y, w, h }, moved by its
   *  camera), or null without one. */
  image: null,
  /** A point of the image (u, v from 0 to 1 across it) in window
   *  coordinates, this frame: [x, y]. Without an image, of the window. */
  fromImage(u, v) {
    const m = this.image;
    return m ? [m.x + u * m.w, m.y + v * m.h] : [u * this.width, v * this.height];
  },
  rect(x, y, w, h, color, alpha = 1, radius = 0) {
    const [c, a] = __color(color);
    if (__plain) return __push(1, x, y, w, h, radius, c, __clamp(alpha * a), 0);
    if (__turned()) return __fillOutline(__rectPoints(x, y, w, h, radius), c, __clamp(alpha * a));
    const x1 = __tx(x, y), y1 = __ty(x, y), x2 = __tx(x + w, y + h), y2 = __ty(x + w, y + h);
    __push(1, Math.min(x1, x2), Math.min(y1, y2), Math.abs(x2 - x1), Math.abs(y2 - y1),
      radius * Math.min(Math.abs(__a), Math.abs(__d)), c, __clamp(alpha * a), 0);
  },
  circle(x, y, r, color, alpha = 1) {
    this.ellipse(x, y, r, r, color, alpha);
  },
  ellipse(x, y, rx, ry, color, alpha = 1) {
    const [c, a] = __color(color);
    if (__plain) return __push(2, x, y, rx, ry, c, __clamp(alpha * a), 0, 0);
    if (__turned()) return __fillOutline(__ellipsePoints(x, y, rx, ry), c, __clamp(alpha * a));
    __push(2, __tx(x, y), __ty(x, y), rx * Math.abs(__a), ry * Math.abs(__d), c, __clamp(alpha * a), 0, 0);
  },
  /** Text centered on (x, y), one monospace cell per character. */
  text(x, y, text, color, alpha = 1, size = 13, bold = false) {
    const [c, a] = __color(color), s = String(text), step = size * 0.6;
    let cx = x - (step * (s.length - 1)) / 2;
    const k = __plain ? 1 : __grow();
    for (const ch of s) {
      if (ch !== ' ') __push(3, __tx(cx, y), __ty(cx, y), size * k, ch.codePointAt(0), c, __clamp(alpha * a), bold ? 1 : 0, 0);
      cx += step;
    }
  },
  /** Pixel art made with pixels(): its top-left at (x, y), each art pixel
   *  `size` window pixels, mirrored left to right when `flip`. Snapped to
   *  whole pixels so its edges stay crisp. */
  pixels(art, x, y, size = 4, alpha = 1, flip = false) {
    if (__plain) { x = Math.round(x); y = Math.round(y); size = Math.max(1, Math.round(size)); }
    for (const [row, col, len, color] of art.runs) {
      const c = flip ? art.w - col - len : col;
      this.rect(x + c * size, y + row * size, len * size, size, color, alpha);
    }
  },
  /** A soft radial glow fading out at radius r. */
  glow(x, y, r, color, alpha = 1) {
    const [c, a] = __color(color);
    __push(4, __tx(x, y), __ty(x, y), __plain ? r : r * __grow(), c, __clamp(alpha * a), 0, 0, 0);
  },
  /** A linear gradient filling the rect; angle in degrees, 180 = top to bottom.
   *  Follows translate and scale, not rotation. */
  gradient(x, y, w, h, angle, from, to, alpha = 1) {
    const [c1, a1] = __color(from), [c2, a2] = __color(to);
    const x1 = __tx(x, y), y1 = __ty(x, y);
    const sx = __plain ? 1 : Math.hypot(__a, __b), sy = __plain ? 1 : Math.hypot(__c, __d);
    __push(5, x1, y1, w * sx, h * sy, angle, c1, c2, __clamp(alpha * a1), __clamp(alpha * a2));
  },
  line(x1, y1, x2, y2, color, alpha = 1, width = 1) {
    const [c, a] = __color(color);
    if (__plain) return __push(6, x1, y1, x2, y2, width, c, __clamp(alpha * a), 0);
    __push(6, __tx(x1, y1), __ty(x1, y1), __tx(x2, y2), __ty(x2, y2), width * __grow(), c, __clamp(alpha * a), 0);
  },
  /** A filled polygon through [x0, y0, x1, y1, …]. */
  poly(points, color, alpha = 1) {
    if (points.length < 6) return;
    const [c, a] = __color(color);
    __fillOutline([points.length >> 1, i => [points[2 * i], points[2 * i + 1]]], c, __clamp(alpha * a));
  },
  /** An open stroke through [x0, y0, x1, y1, …]. */
  polyline(points, color, alpha = 1, width = 1) {
    if (points.length < 4) return;
    const [c, a] = __color(color), start = __np / 2;
    for (let i = 0; i + 1 < points.length; i += 2) __pt(__tx(points[i], points[i + 1]), __ty(points[i], points[i + 1]));
    __pt(NaN, 0);
    __push(7, start, __np / 2 - start, Math.max(0.5, width * __grow()), c, __clamp(alpha * a), 0, 0, 0);
  },
  /** A path to build once and draw many times (see __Path). */
  path() { return new __Path(); },
  save() { __saved.push([__a, __b, __c, __d, __e, __f]); },
  restore() {
    const m = __saved.pop();
    if (m) { [__a, __b, __c, __d, __e, __f] = m; __settle(); }
  },
  resetTransform() { __a = 1; __b = 0; __c = 0; __d = 1; __e = 0; __f = 0; __plain = true; },
  translate(x, y) { __e += __a * x + __c * y; __f += __b * x + __d * y; __settle(); },
  scale(sx, sy = sx) { __a *= sx; __b *= sx; __c *= sy; __d *= sy; __settle(); },
  rotate(angle) {
    const cos = Math.cos(angle), sin = Math.sin(angle);
    [__a, __b, __c, __d] = [__a * cos + __c * sin, __b * cos + __d * sin, __c * cos - __a * sin, __d * cos - __b * sin];
    __settle();
  },
};

// A path in its own coordinates: curves are kept as such and flattened when
// the path is drawn, finely enough for the scale it's drawn at.
class __Path {
  constructor() { this.ops = []; this.open = false; this.x = 0; this.y = 0; }
  moveTo(x, y) { this.ops.push([0, x, y]); this.open = true; this.x = x; this.y = y; return this; }
  lineTo(x, y) {
    if (!this.open) return this.moveTo(x, y);
    this.ops.push([1, x, y]); this.x = x; this.y = y; return this;
  }
  quadTo(cx, cy, x, y) {
    if (!this.open) this.moveTo(this.x, this.y);
    this.ops.push([2, this.x, this.y, cx, cy, x, y]); this.x = x; this.y = y; return this;
  }
  curveTo(c1x, c1y, c2x, c2y, x, y) {
    if (!this.open) this.moveTo(this.x, this.y);
    this.ops.push([3, this.x, this.y, c1x, c1y, c2x, c2y, x, y]); this.x = x; this.y = y; return this;
  }
  /** An arc around (cx, cy) from angle `from` to `to` (radians), joined to the current point. */
  arc(cx, cy, r, from, to) {
    const x = cx + Math.cos(from) * r, y = cy + Math.sin(from) * r;
    if (this.open) this.lineTo(x, y); else this.moveTo(x, y);
    this.ops.push([4, cx, cy, r, from, to]);
    this.x = cx + Math.cos(to) * r; this.y = cy + Math.sin(to) * r;
    return this;
  }
  ellipse(cx, cy, rx, ry) {
    this.ops.push([5, cx, cy, rx, ry]); this.open = false; this.x = cx + rx; this.y = cy; return this;
  }
  rect(x, y, w, h) {
    this.moveTo(x, y).lineTo(x + w, y).lineTo(x + w, y + h).lineTo(x, y + h).close();
    this.x = x; this.y = y; return this;
  }
  close() { if (this.open) { this.ops.push([6]); this.open = false; } return this; }
  // The path's points in the current transform; returns the first pair.
  __emit() {
    const start = __np / 2, k = __grow();
    let inside = false;
    const end = closed => { if (inside) { __pt(NaN, closed ? 1 : 0); inside = false; } };
    const put = (x, y) => { __pt(__tx(x, y), __ty(x, y)); inside = true; };
    const steps = len => Math.max(4, Math.min(48, Math.ceil((len * k) / 6)));
    for (const op of this.ops) {
      switch (op[0]) {
        case 0: end(false); put(op[1], op[2]); break;
        case 1: put(op[1], op[2]); break;
        case 2: {
          const [, x0, y0, cx, cy, x, y] = op;
          const n = steps(Math.hypot(cx - x0, cy - y0) + Math.hypot(x - cx, y - cy));
          for (let i = 1; i <= n; i++) {
            const s = i / n, u = 1 - s;
            put(u * u * x0 + 2 * u * s * cx + s * s * x, u * u * y0 + 2 * u * s * cy + s * s * y);
          }
          break;
        }
        case 3: {
          const [, x0, y0, ax, ay, bx, by, x, y] = op;
          const n = steps(Math.hypot(ax - x0, ay - y0) + Math.hypot(bx - ax, by - ay) + Math.hypot(x - bx, y - by));
          for (let i = 1; i <= n; i++) {
            const s = i / n, u = 1 - s;
            put(u * u * u * x0 + 3 * u * u * s * ax + 3 * u * s * s * bx + s * s * s * x,
              u * u * u * y0 + 3 * u * u * s * ay + 3 * u * s * s * by + s * s * s * y);
          }
          break;
        }
        case 4: {
          const [, cx, cy, r, from, to] = op;
          const n = steps(Math.abs(to - from) * r);
          for (let i = 1; i <= n; i++) {
            const a = from + ((to - from) * i) / n;
            put(cx + Math.cos(a) * r, cy + Math.sin(a) * r);
          }
          break;
        }
        case 5: {
          end(false);
          const [, cx, cy, rx, ry] = op;
          const n = Math.max(12, steps(Math.PI * (rx + ry)));
          for (let i = 0; i < n; i++) { const a = (i / n) * Math.PI * 2; put(cx + Math.cos(a) * rx, cy + Math.sin(a) * ry); }
          end(true);
          break;
        }
        case 6: end(true); break;
      }
    }
    end(false);
    return start;
  }
  fill(color, alpha = 1) {
    const [c, a] = __color(color), start = this.__emit();
    if (__np / 2 > start) __push(7, start, __np / 2 - start, 0, c, __clamp(alpha * a), 0, 0, 0);
    return this;
  }
  stroke(color, alpha = 1, width = 1) {
    const [c, a] = __color(color), start = this.__emit();
    if (__np / 2 > start) __push(7, start, __np / 2 - start, Math.max(0.5, width * __grow()), c, __clamp(alpha * a), 0, 0, 0);
    return this;
  }
  /** A linear gradient over the path's bounds; 180 = top to bottom. */
  fillGradient(angle, from, to, alpha = 1) {
    const [c1, a1] = __color(from), [c2, a2] = __color(to), start = this.__emit();
    if (__np / 2 > start) __push(8, start, __np / 2 - start, angle, c1, c2, __clamp(alpha * a1), __clamp(alpha * a2), 0);
    return this;
  }
}

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

// ── Smooth motion: noise and small math helpers ────────────────────────────
const TAU = Math.PI * 2;
const lerp = (a, b, k) => a + (b - a) * k;
const clamp = (v, lo, hi) => (v < lo ? lo : v > hi ? hi : v);
const smoothstep = (e0, e1, x) => { const k = clamp((x - e0) / (e1 - e0), 0, 1); return k * k * (3 - 2 * k); };
const __perm = new Uint8Array(512);
{
  const p = Array.from({ length: 256 }, (_, i) => i);
  for (let i = 255; i > 0; i--) { const j = Math.floor(hash(i, 7, 13) * (i + 1)); [p[i], p[j]] = [p[j], p[i]]; }
  for (let i = 0; i < 512; i++) __perm[i] = p[i & 255];
}
const __fade = k => k * k * k * (k * (k * 6 - 15) + 10);
const __gradDot = (h, x, y, z) => {
  const u = h < 8 ? x : y, v = h < 4 ? y : h === 12 || h === 14 ? x : z;
  return ((h & 1) ? -u : u) + ((h & 2) ? -v : v);
};
/** Smooth noise in -1…1: nearby inputs give nearby values (Perlin). The
 *  inputs are nudged off the lattice, where Perlin noise is always 0, so
 *  whole numbers (an index, t at a whole second) still vary. */
const noise = (x, y, z = 0) => {
  x += 0.3183; y += 0.1781; z += 0.4142;
  const X = Math.floor(x) & 255, Y = Math.floor(y) & 255, Z = Math.floor(z) & 255;
  x -= Math.floor(x); y -= Math.floor(y); z -= Math.floor(z);
  const u = __fade(x), v = __fade(y), w = __fade(z), P = __perm;
  const A = P[X] + Y, AA = P[A] + Z, AB = P[A + 1] + Z, B = P[X + 1] + Y, BA = P[B] + Z, BB = P[B + 1] + Z;
  return lerp(
    lerp(lerp(__gradDot(P[AA] & 15, x, y, z), __gradDot(P[BA] & 15, x - 1, y, z), u),
      lerp(__gradDot(P[AB] & 15, x, y - 1, z), __gradDot(P[BB] & 15, x - 1, y - 1, z), u), v),
    lerp(lerp(__gradDot(P[AA + 1] & 15, x, y, z - 1), __gradDot(P[BA + 1] & 15, x - 1, y, z - 1), u),
      lerp(__gradDot(P[AB + 1] & 15, x, y - 1, z - 1), __gradDot(P[BB + 1] & 15, x - 1, y - 1, z - 1), u), v),
    w);
};
/** Layered noise (roughly -1…1): clouds, water, wind. */
const fbm = (x, y, z = 0, octaves = 4) => {
  let sum = 0, amp = 1, norm = 0, f = 1;
  for (let i = 0; i < octaves; i++) { sum += amp * noise(x * f, y * f, z * f + i * 17.3); norm += amp; amp *= 0.5; f *= 2; }
  return sum / norm;
};
/** A path to build once at the top of the file and draw many times. */
const path = () => new __Path();
/** Pixel art from rows of characters, one per pixel, each looked up in
 *  `palette` (a character to a color); '.' and ' ' are see-through. Build it
 *  once at the top of the file and draw it with g.pixels(art, x, y, size). */
const pixels = (rows, palette) => {
  const runs = [];
  rows.forEach((line, row) => {
    for (let col = 0; col < line.length;) {
      const ch = line[col];
      let len = 1;
      while (line[col + len] === ch) len++;
      if (ch !== '.' && ch !== ' ' && palette[ch] !== undefined) runs.push([row, col, len, palette[ch]]);
      col += len;
    }
  });
  return { w: Math.max(0, ...rows.map(r => r.length)), h: rows.length, runs };
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
const __frame = (W, H, t, cell, image) => {
  __n = 0; __np = 0;
  __g.resetTransform(); __saved.length = 0;
  __g.width = W; __g.height = H; __g.t = t;
  __g.image = image ? { x: image[0], y: image[1], w: image[2], h: image[3] } : null;
  const most = typeof __scene.size === 'number' ? __scene.size : 22;
  CELL = Math.max(8, Math.min(cell, most));
  __glyphSize = Math.round(CELL * 13 / 22);
  if (__scene.draw) __scene.draw(__g, t);
  __g.resetTransform();
  if (__scene.cell || __scene.sprites) __ascii(__scene, W, H, t);
  return __n;
};

// (top-level const/let aren't properties of globalThis: this is how the app reaches them)
globalThis.__api = {
  pick: __pick,
  frame: __frame,
  out: () => __buf,
  points: () => __pts,
  pointCount: () => __np / 2,
  setTheme: theme => { __g.theme = theme; },
  setState: (state, event) => { root.dataset.themeState = state || undefined; root.dataset.themeEvent = event || undefined; },
};
