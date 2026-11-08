// Run from the repo root: node scripts/gen-logo.js (then regenerate assets/app.ico with --write-icon).
// Generates the "Hub ring" logo SVGs (owner pick C, 2026-11-08): a centre with three
// coloured arcs around it (your accounts around one hub).
const fs = require("fs");
const dir = "crates/subhub-app/assets/";
const f = (v) => +v.toFixed(2);
const P = (r, a) => [32 + r * Math.cos((a * Math.PI) / 180), 32 + r * Math.sin((a * Math.PI) / 180)];
const arc = (r, a0, a1) => {
  const [x0, y0] = P(r, a0), [x1, y1] = P(r, a1);
  return `M${f(x0)} ${f(y0)} A${r} ${r} 0 0 1 ${f(x1)} ${f(y1)}`;
};
const grads = `<linearGradient id="o" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ff8a5c"/><stop offset="1" stop-color="#e2552a"/></linearGradient>
    <linearGradient id="b" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#38bdf8"/><stop offset="1" stop-color="#2563eb"/></linearGradient>
    <linearGradient id="y" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#facc15"/><stop offset="1" stop-color="#f59e0b"/></linearGradient>`;
// Arcs: orange top right, blue bottom, yellow left; gaps centred at the top, lower right and
// lower left. `span` is each arc's angle, the rest is gap (round caps eat stroke/2 per end).
function ring({ r, stroke, span, centre, ink }) {
  const starts = [-30, 90, 210].map((c) => [c - span / 2, c + span / 2]);
  const ids = ["o", "b", "y"];
  return `  <g fill="none" stroke-width="${stroke}" stroke-linecap="round">
${starts.map(([a, b], i) => `    <path d="${arc(r, a, b)}" stroke="url(#${ids[i]})"/>`).join("\n")}
  </g>
  <circle cx="32" cy="32" r="${centre}" fill="${ink}"/>`;
}
const head = (comment) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">\n  <!-- ${comment} -->\n`;
function tile(comment, dark, ringOpts) {
  const [a, b] = dark ? ["#191d3a", "#0b0e1e"] : ["#ffffff", "#e6e9f8"];
  const edge = dark
    ? `<rect x="1.5" y="1.5" width="61" height="61" rx="13.5" fill="none" stroke="#ffffff" stroke-opacity=".16"/>`
    : `<rect x="1.5" y="1.5" width="61" height="61" rx="13.5" fill="none" stroke="#1c2350" stroke-opacity=".18"/>`;
  return (
    head(comment) +
    `  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient>
    ${grads}
  </defs>
  <rect x="1" y="1" width="62" height="62" rx="14" fill="url(#bg)"/>
${ring({ ...ringOpts, ink: dark ? "#eef1ff" : "#1c2350" })}
  ${edge}
</svg>
`
  );
}
function bare(comment, ink) {
  return (
    head(comment) +
    `  <defs>
    ${grads}
  </defs>
${ring({ r: 22, stroke: 12, span: 64, centre: 9, ink })}
</svg>
`
  );
}
const big = { r: 19, stroke: 9, span: 72, centre: 7.5 };
const small = { r: 18.5, stroke: 10.5, span: 64, centre: 8 };
fs.writeFileSync(dir + "logo.svg", tile('AI SubHub app icon "Hub ring" (owner pick, 2026-11-08): a centre with three coloured arcs, your accounts around one hub. Dark tile, 48 px and up. Generated; static, never live values.', true, big));
fs.writeFileSync(dir + "logo-small.svg", tile("Small master (16-32 px) of the Hub ring app icon: thicker arcs and wider gaps. Dark tile, for light backgrounds.", true, small));
fs.writeFileSync(dir + "tray-dark.svg", bare("Hub ring bare mark (tray, title bar, popover header) for dark backgrounds: no tile, light centre.", "#ffffff"));
fs.writeFileSync(dir + "tray-light.svg", bare("Hub ring bare mark (tray, title bar, popover header) for light backgrounds: no tile, dark centre.", "#1c2350"));
