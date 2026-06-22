/**
 * Rolls-Royce–inspired "Power Reserve" circular gauge.
 *
 * Flat-friendly design: no drop shadows or specular highlights.
 * Chrome bezel as a simple silver band, Prussian-blue accent ring,
 * clean white face, slim tapered needle.
 *
 * Arc: 8 o'clock → 12 → 3 o'clock (210° sweep).
 * Value: 0–5 float from pwrzv.
 */

interface PowerReserveGaugeProps {
  value: number;
  size?: number;
}

const CX = 200;
const CY = 200;
const R = 128;
const ARC_START_DEG = 150;
const ARC_SWEEP_DEG = 210;

function degToRad(d: number) {
  return (d * Math.PI) / 180;
}

function p(angleDeg: number, r: number) {
  const rad = degToRad(angleDeg);
  return { x: CX + r * Math.cos(rad), y: CY + r * Math.sin(rad) };
}

function valueToDeg(v: number) {
  return ARC_START_DEG + (Math.max(0, Math.min(5, v)) / 5) * ARC_SWEEP_DEG;
}

function arcPath(r: number, startDeg: number, endDeg: number) {
  const s = p(startDeg, r);
  const e = p(endDeg, r);
  return `M ${s.x} ${s.y} A ${r} ${r} 0 ${endDeg - startDeg > 180 ? 1 : 0} 1 ${e.x} ${e.y}`;
}

export function PowerReserveGauge({ value, size = 340 }: PowerReserveGaugeProps) {
  const v = Math.max(0, Math.min(5, value));
  const needleDeg = valueToDeg(v);

  const gradS = p(ARC_START_DEG, R);
  const gradE = p(ARC_START_DEG + ARC_SWEEP_DEG, R);

  const ticks: { deg: number; major: boolean; label?: string }[] = [];
  for (let i = 0; i <= 10; i++) {
    const val = i * 0.5;
    const deg = valueToDeg(val);
    const major = i % 2 === 0;
    ticks.push({ deg, major, label: major ? String(val) : undefined });
  }

  const fullArcD = arcPath(R, ARC_START_DEG, ARC_START_DEG + ARC_SWEEP_DEG);

  // Needle geometry — straight line, round caps
  const tipP = p(needleDeg, R - 10);
  const tailP = p(needleDeg + 180, 18);

  return (
    <svg viewBox="0 0 400 400" width={size} height={size} className="mx-auto">
      <defs>
        {/* Arc colour scale */}
        <linearGradient
          id="prg-arc"
          gradientUnits="userSpaceOnUse"
          x1={gradS.x} y1={gradS.y}
          x2={gradE.x} y2={gradE.y}
        >
          <stop offset="0%" stopColor="#b91c1c" />
          <stop offset="6%" stopColor="#dc2626" />
          <stop offset="12%" stopColor="#e87a20" />
          <stop offset="20%" stopColor="#eab308" />
          <stop offset="32%" stopColor="#a3c520" />
          <stop offset="45%" stopColor="#22c55e" />
          <stop offset="100%" stopColor="#166534" />
        </linearGradient>
      </defs>

      {/* ── Bezel — flat silver band ── */}
      <circle cx={CX} cy={CY} r={180} fill="#eff3fb" stroke="#d5ddf0" strokeWidth={1} opacity={0.9} />
      <circle cx={CX} cy={CY} r={173} fill="#eff3fb" stroke="#d5ddf0" strokeWidth={1} opacity={0.9} />

      {/* ── Blue accent ring ── */}
      <circle cx={CX} cy={CY} r={168} fill="none" stroke="#1a3a6a" strokeWidth={5} />

      {/* ── Dial face — clean white ── */}
      <circle cx={CX} cy={CY} r={165.5} fill="#fcfcfc" />

      {/* ── Colour arc ── */}
      <path
        d={fullArcD}
        fill="none"
        stroke="url(#prg-arc)"
        strokeWidth={7}
        strokeLinecap="round"
      />

      {/* ── Tick marks ── */}
      {ticks.map((t, i) => {
        const outerR = R + 13;
        const innerR = t.major ? R + 3 : R + 7;
        const o = p(t.deg, outerR);
        const n = p(t.deg, innerR);
        const lp = p(t.deg, R + 26);
        return (
          <g key={i}>
            <line
              x1={o.x} y1={o.y} x2={n.x} y2={n.y}
              stroke={t.major ? "#2a2a30" : "#a8a8b0"}
              strokeWidth={t.major ? 1.5 : 0.6}
            />
            {t.label != null && (
              <text
                x={lp.x}
                y={lp.y}
                textAnchor="middle"
                dominantBaseline="central"
                fill="#2a2a30"
                fontSize={12.5}
                fontWeight={400}
                fontFamily="'Georgia', 'Times New Roman', serif"
              >
                {t.label}
              </text>
            )}
          </g>
        );
      })}

      {/* ── Needle ── */}
      <line
        x1={tailP.x} y1={tailP.y}
        x2={tipP.x} y2={tipP.y}
        stroke="#1a1a22"
        strokeWidth={2}
        strokeLinecap="round"
      />

      {/* ── Centre emblem ── */}
      <circle cx={CX} cy={CY} r={28} fill="#fcfcfc" stroke="#1a3a6a" strokeWidth={1.5} />
      <text
        x={CX}
        y={CY + 1}
        textAnchor="middle"
        dominantBaseline="central"
        fill="#1a3a6a"
        fontSize={26}
        fontWeight={400}
        fontFamily="'Didot', 'Bodoni MT', 'Noto Serif Display', 'Playfair Display', serif"
      >
        A
      </text>

      {/* ── POWER RESERVE readout ── */}
      <text
        x={CX + 50}
        y={CY + 86}
        textAnchor="middle"
        fill="#8a8a94"
        fontSize={10}
        fontFamily="'Didot', 'Bodoni MT', 'Noto Serif Display', 'Playfair Display', serif"
        letterSpacing={3}
      >
        POWER RESERVE
      </text>
      <text
        x={CX + 50}
        y={CY + 110}
        textAnchor="middle"
        fontFamily="'Didot', 'Bodoni MT', 'Noto Serif Display', 'Playfair Display', serif"
      >
        <tspan fontWeight={500} fill="#1a1a22" fontSize={20}>{v.toFixed(1)}</tspan>
        <tspan fill="#8a8a94" fontSize={13}> /5</tspan>
      </text>
    </svg>
  );
}
