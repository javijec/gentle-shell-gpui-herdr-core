// Petals: cherry petals tumbling down on a breeze, a few catching the light,
// over a soft glow in the corners. A pixel scene (draw(g, t) with g.*).

const PETAL_COLORS = ['#ffc9dd', '#f7a8c8', '#ffe1ec', '#f095c8'];

SCENES.petals = {
  draw(g, t) {
    const W = g.width, H = g.height;
    g.glow(0, 0, Math.max(W, H) * .45, '#f095c8', .22);
    g.glow(W, H, Math.max(W, H) * .4, '#b48cff', .16);
    const count = Math.round(Math.min(90, 30 + W * H / 30000));
    for (let i = 0; i < count; i++) {
      const fall = 18 + 26 * hash(i, 1), travel = H + 60;
      const y = (t * fall + hash(i, 2) * travel) % travel - 30;
      const breeze = Math.sin(t * .2) * 40;
      const sway = 26 * Math.sin(t * (.4 + .5 * hash(i, 3)) + i);
      const x = ((hash(i, 4) * (W + 80) + sway + breeze + t * 8) % (W + 80)) - 40;
      // Tumbling: the petal's width breathes as it turns.
      const turn = Math.abs(Math.sin(t * (.6 + hash(i, 5)) + i * 1.7));
      const size = 5 + 6 * hash(i, 6);
      const color = PETAL_COLORS[i % PETAL_COLORS.length];
      g.ellipse(x, y, size * (.35 + .65 * turn), size * .6, color, .35 + .4 * hash(i, 7));
      if (turn > .97) g.glow(x, y, size * 3, '#ffffff', .25);
    }
  },
};
