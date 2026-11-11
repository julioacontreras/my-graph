/** Decorative background graph with the blue → violet glow. */
const NODES: [number, number, number, string][] = [
  [80, 120, 5, "#1a8bff"], [210, 60, 4, "#4d9bff"], [330, 150, 6, "#006cfc"], [460, 70, 3.5, "#8b2cf5"],
  [560, 190, 5, "#2f86ff"], [700, 110, 4, "#8b2cf5"], [820, 210, 6, "#006cfc"], [960, 90, 4, "#4d9bff"],
  [1080, 180, 5, "#8b2cf5"], [150, 300, 4, "#8b2cf5"], [420, 320, 4.5, "#1a8bff"], [640, 360, 3.5, "#8b2cf5"],
  [900, 380, 4.5, "#2f86ff"], [1120, 330, 4, "#006cfc"], [260, 470, 3.5, "#2f86ff"], [760, 500, 4, "#8b2cf5"],
  [1000, 520, 3.5, "#4d9bff"], [520, 520, 3, "#006cfc"],
];
const EDGES: [number, number][] = [
  [0, 1], [1, 2], [0, 2], [2, 3], [3, 4], [2, 4], [4, 5], [5, 6], [4, 6], [6, 7], [7, 8], [6, 8],
  [0, 9], [9, 10], [2, 10], [10, 11], [4, 11], [11, 12], [6, 12], [12, 13], [8, 13], [9, 14], [14, 10],
  [11, 15], [15, 12], [12, 16], [16, 13], [14, 17], [17, 11], [17, 15],
];

export function HeroGraph() {
  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden" aria-hidden="true">
      <div className="pulse-glow absolute -left-40 top-10 size-[520px] rounded-full bg-primary/25 blur-[120px]" />
      <div className="pulse-glow absolute -right-32 top-40 size-[480px] rounded-full bg-violet/20 blur-[120px]" style={{ animationDelay: "-3s" }} />
      <svg className="absolute inset-0 h-full w-full opacity-30 [mask-image:linear-gradient(to_right,transparent,black_15%,black_85%,transparent)]" viewBox="0 0 1200 600" preserveAspectRatio="xMidYMid slice">
        <g className="drift">
          {EDGES.map(([a, b], i) => (
            <line key={i} x1={NODES[a][0]} y1={NODES[a][1]} x2={NODES[b][0]} y2={NODES[b][1]} stroke="#29364d" strokeWidth="1" />
          ))}
        </g>
        <g className="drift">
          {NODES.map(([x, y, r, c], i) => (
            <circle key={i} cx={x} cy={y} r={r} fill={c} opacity={0.85} />
          ))}
        </g>
      </svg>
      <div className="absolute inset-x-0 bottom-0 h-40 bg-gradient-to-b from-transparent to-bg" />
    </div>
  );
}
