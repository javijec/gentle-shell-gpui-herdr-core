// Aurora: soft curtains of light swaying over a starry night, with a slow
// glow drifting along the horizon. A pixel scene: it draws with g.* in
// draw(g, t) instead of ASCII cells.

const CURTAINS = [
  // [height share, sway speed, phase, color]
  [.42, .11, 0, '#5cf2b8'],
  [.36, .08, 2.1, '#7aa8ff'],
  [.30, .14, 4.2, '#b48cff'],
];

SCENES.aurora = {
  draw(g, t) {
    const W = g.width, H = g.height;
    // Stars: fixed spots that twinkle.
    for (let i = 0; i < 140; i++) {
      const x = hash(i, 1) * W, y = hash(i, 2) * H * .7;
      const tw = .35 + .65 * (.5 + .5 * Math.sin(t * (.8 + hash(i, 3) * 2) + i));
      g.circle(x, y, .6 + hash(i, 4) * 1.1, '#e8f0ff', .55 * tw);
    }
    // Curtains: narrow overlapping streaks whose tops ripple, each fading in
    // at the top and out toward the bottom.
    const step = Math.max(3, W / 360);
    CURTAINS.forEach(([share, speed, phase, color], k) => {
      for (let x = -step; x < W + step; x += step) {
        const u = x / W;
        const wave = Math.sin(u * 5 + t * speed * 6 + phase) * .5 + Math.sin(u * 9 - t * speed * 4 + phase * 2) * .2;
        const top = H * (.08 + .1 * k + .08 * wave);
        const len = H * share * (.75 + .25 * Math.sin(u * 3 + t * .3 + phase));
        const bright = .5 + .5 * Math.sin(u * 3.5 + t * speed * 7 + phase);
        const a = .1 + .14 * bright, head = len * .18;
        g.gradient(x, top - head, step * 2, head, 180, color + '00', color, a);
        g.gradient(x, top, step * 2, len, 180, color, color + '00', a);
      }
    });
    // A warm glow drifting along the horizon.
    const gx = W * (.5 + .4 * Math.sin(t * .05));
    g.glow(gx, H * .95, Math.max(W, H) * .35, g.theme.accent || '#f095c8', .25);
  },
};
