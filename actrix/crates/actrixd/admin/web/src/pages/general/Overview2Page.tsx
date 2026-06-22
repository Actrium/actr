import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { usePlatformData, filterFields, PlatformDataGuard } from "../platform/shared";
import { ServicePageLayout, ConfigSection } from "../../components/layout/ServicePageLayout";
import type { ResolvedField } from "../../lib/api";

const VIEW_W = 1060;
const VIEW_H = 480;
const FONT = "ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif";

function getField(fields: ResolvedField[], key: string): string {
  const f = fields.find((x) => x.key === key);
  return f ? f.effective_value || "—" : "—";
}

function short(uri: string, max = 32): string {
  if (uri.length <= max) return uri;
  return uri.slice(0, Math.max(0, max - 3)) + "...";
}

const nodeSections = [
  {
    title: "Node Identity",
    desc: "Instance name, environment, and database path",
    keys: ["enable", "name", "env", "location_tag", "sqlite_path"],
  },
  {
    title: "HTTP Binding",
    desc: "Foundation for Signaling, AIS, Signer, and Control",
    keys: ["bind.http"],
  },
  {
    title: "ICE Binding",
    desc: "Foundation for STUN and TURN",
    keys: ["bind.ice", "turn.relay_port_range"],
  },
  {
    title: "Control Plane",
    desc: "Control plane mode and admin session settings",
    keys: ["control"],
  },
];

export function Overview2Page() {
  const navigate = useNavigate();
  const [hovered, setHovered] = useState<string | null>(null);
  const { data, error, fetchData } = usePlatformData();

  return (
    <PlatformDataGuard data={data} error={error}>
      {(d) => {
        const enableMask = parseInt(getField(d.config_fields, "enable"), 10) || 0;
        const httpPort = getField(d.config_fields, "bind.http.port");
        const icePort = getField(d.config_fields, "bind.ice.port");


        // Recording data
        const globalSink = (() => {
          const f = d.config_fields.find((x) => x.key === "recording.sink");
          return f ? f.effective_value.trim() || "stderr" : "stderr";
        })();
        const channelDefs = [
          { id: "observability", label: "Observability", color: "#0369a1", bg: "#e0f2fe" },
          { id: "audit", label: "Audit", color: "#b45309", bg: "#fef3c7" },
          { id: "security", label: "Security", color: "#be185d", bg: "#fce7f3" },
          { id: "operations", label: "Operations", color: "#4338ca", bg: "#e0e7ff" },
        ];
        const channelData = channelDefs.map((ch) => {
          const filter = getField(d.config_fields, `recording.${ch.id}.filter`);
          const perSink = getField(d.config_fields, `recording.${ch.id}.sink`);
          const sink = perSink !== "—" ? perSink : globalSink;
          const sinkType = sink.startsWith("file://") ? "file" : sink.startsWith("otlp+") ? "otlp" : "stderr";
          return { ...ch, filter, sink, sinkType, hasOverride: perSink !== "—" };
        });

        const layerCounts = { default: 0, config_file: 0, override: 0 };
        for (const f of d.config_fields) {
          if (f.source === "config_file") layerCounts.config_file++;
          else if (f.source === "override") layerCounts.override++;
          else layerCounts.default++;
        }

        const ENABLE_BITS: Record<string, number> = {
          Signaling: 1, STUN: 2, TURN: 4, AIS: 8, Signer: 16,
        };
        const serviceOn = (name: string) =>
          (enableMask & (ENABLE_BITS[name] ?? 0)) !== 0 || name === "Control";

        const pillRoutes: Record<string, string> = {
          Admin: "/admin", Control: "/admin/services/control", Signer: "/admin/services/signer",
          AIS: "/admin/services/ais", MFR: "/admin/mfr", Signaling: "/admin/services/signaling",
          STUN: "/admin/services/stun", TURN: "/admin/services/turn",
        };
        const pillHoverStrokes: Record<string, string> = {
          Admin: "#6366f1", Control: "#d97706", Signer: "#d97706",
          AIS: "#16a34a", MFR: "#16a34a", Signaling: "#16a34a", STUN: "#16a34a", TURN: "#16a34a",
        };
        const pill = (x: number, y: number, label: string, on: boolean, w = 84, colors?: { fill: string; stroke: string; text: string }) => {
          const fill = colors?.fill ?? (on ? "#ecfdf5" : "#f8fafc");
          const text = colors?.text ?? (on ? "#166534" : "#94a3b8");
          const hoverKey = `pill-${label}`;
          const route = pillRoutes[label];
          const interactive = !!route && label !== "Control";
          return (
            <g key={`${label}-${x}-${y}`} style={interactive ? { cursor: "pointer" } : undefined} onClick={interactive ? () => navigate(route) : undefined} onMouseEnter={interactive ? () => setHovered(hoverKey) : undefined} onMouseLeave={interactive ? () => setHovered(null) : undefined}>
              <rect x={x} y={y} width={w} height={44} rx={8} fill={fill} stroke={hovered === hoverKey && interactive ? (pillHoverStrokes[label] ?? "#94a3b8") : "none"} strokeWidth={1.5} />
              <text x={x + w / 2} y={y + 20} textAnchor="middle" fontSize={11} fontWeight={700} fill={text}>
                {label}
              </text>
            </g>
          );
        };
        const controlColors = { fill: "#eef2ff", stroke: "#6366f1", text: "#3730a3" };
        const signerColors = { fill: "#fef3c7", stroke: "#d97706", text: "#92400e" };

        return (
          <ServicePageLayout
            title="System Architecture"
            description="Top-down view: external clients, protocol/service layer, infrastructure subsystems."
          >
            <div>
              <svg
                viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
                className="w-full"
                xmlns="http://www.w3.org/2000/svg"
                fontFamily={FONT}
              >
                {/* Grid: left=20, right=1040, usable=1020, gap=20 */}

                {/* ═══════════════════════════════════════════ */}
                {/* ROW 1: External Clients  y=10 h=70         */}
                {/* 3 boxes ratio 2:4:1, gap=20                */}
                {/* w=280/560/140, x=20/320/900                */}
                {/* ═══════════════════════════════════════════ */}
                {/* All icons: 16×16 box, stroke=#64748b, strokeWidth=1.2, fill=none */}

                {/* Layout: icon 32×32 at x=16 y=24, text from x=60: title y=34, note y=52 */}

                {/* Admin | Supervisor  (w=252) */}
                <g transform="translate(20,10)">
                  <rect width={252} height={80} rx={10} fill="#fafbfc" stroke="#e2e8f0" />
                  {/* Two-person silhouette icon 32×32 */}
                  <g transform="translate(16,24)">
                    {/* Front person */}
                    <rect x={6} y={0} width={10} height={10} rx={3} fill="none" stroke="#94a3b8" strokeWidth={1.6} />
                    <rect x={2} y={14} width={18} height={14} rx={4} fill="none" stroke="#94a3b8" strokeWidth={1.6} />
                    {/* Back person (offset right, slightly behind) */}
                    <rect x={20} y={2} width={8} height={8} rx={2.5} fill="none" stroke="#94a3b8" strokeWidth={1.4} />
                    <rect x={18} y={14} width={14} height={12} rx={3.5} fill="none" stroke="#94a3b8" strokeWidth={1.4} />
                  </g>
                  <text x={60} y={34} textAnchor="start" fontSize={12} fontWeight={600} fill="#475569">Admin</text>
                  <text x={108} y={35} textAnchor="middle" fontSize={16} fontWeight={300} fill="#b0bec5">|</text>
                  <text x={118} y={34} textAnchor="start" fontSize={12} fontWeight={600} fill="#475569">Supervisor</text>
                  <text x={60} y={52} textAnchor="start" fontSize={9} fill="#94a3b8">Admin Web UI · Control gRPC API</text>
                </g>

                {/* Actor Peers  (w=560) */}
                <g transform="translate(292,10)">
                  <rect width={560} height={80} rx={10} fill="#fafbfc" stroke="#e2e8f0" />
                  {/* Honeycomb icon 32×32 */}
                  <g transform="translate(16,24)">
                    <polygon points="16,0 24,4.5 24,13.5 16,18 8,13.5 8,4.5" fill="none" stroke="#94a3b8" strokeWidth={1.6} />
                    <polygon points="7,16 15,20.5 15,29.5 7,34 -1,29.5 -1,20.5" fill="none" stroke="#94a3b8" strokeWidth={1.6} />
                    <polygon points="25,16 33,20.5 33,29.5 25,34 17,29.5 17,20.5" fill="none" stroke="#94a3b8" strokeWidth={1.6} />
                  </g>
                  <text x={60} y={34} textAnchor="start" fontSize={12} fontWeight={600} fill="#475569">Actor Peers</text>
                  <text x={60} y={52} textAnchor="start" fontSize={9} fill="#94a3b8">WS Signaling · Credentials · Realm Discovery / ACL · Capability Negotiation · STUN · TURN</text>
                </g>

                {/* Metrics  (w=168) */}
                <g transform="translate(872,10)">
                  <rect width={168} height={80} rx={10} fill="#fafbfc" stroke="#e2e8f0" />
                  {/* Bar chart icon 32×32 */}
                  <g transform="translate(16,24)">
                    <rect x={1} y={20} width={8} height={12} rx={1.5} fill="none" stroke="#94a3b8" strokeWidth={1.6} />
                    <rect x={12} y={10} width={8} height={22} rx={1.5} fill="none" stroke="#94a3b8" strokeWidth={1.6} />
                    <rect x={23} y={0} width={8} height={32} rx={1.5} fill="none" stroke="#94a3b8" strokeWidth={1.6} />
                  </g>
                  <text x={60} y={34} textAnchor="start" fontSize={12} fontWeight={600} fill="#475569">Metrics</text>
                  <text x={60} y={52} textAnchor="start" fontSize={9} fill="#94a3b8">/health · /metrics</text>
                </g>

                {/* ═══════════════════════════════════════════ */}
                {/* ROW 2: Services  y=106 h=110               */}
                {/* 1 box w=1020, 8 pills w=112, gap=12        */}
                {/* ═══════════════════════════════════════════ */}

                <g transform="translate(20,106)">
                  <rect width={1020} height={110} rx={14} fill="none" stroke="#94a3b8" strokeWidth={1.2} opacity={0.85} />
                  <text x={16} y={22} textAnchor="start" fontSize={11} fontWeight={800} fill="#334155">
                    Services
                  </text>

                  {/* Protocol group labels */}
                  <line x1={16} y1={34} x2={748} y2={34} stroke="#cbd5e1" strokeWidth={0.8} />
                  <text x={382} y={30} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#94a3b8">http+websocket/{httpPort}</text>
                  <line x1={760} y1={34} x2={998} y2={34} stroke="#cbd5e1" strokeWidth={0.8} />
                  <text x={878} y={30} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#94a3b8">udp/{icePort}</text>

                  {/* 8 pills: w=112, gap=12 */}
                  {pill(16, 41, "Admin", true, 112, controlColors)}
                  {pill(140, 41, "Control", true, 112, signerColors)}
                  {pill(264, 41, "Signer", serviceOn("Signer"), 112, signerColors)}
                  {pill(388, 41, "AIS", serviceOn("AIS"), 112)}
                  {pill(512, 41, "MFR", true, 112)}
                  {pill(636, 41, "Signaling", serviceOn("Signaling"), 112)}
                  {pill(760, 41, "STUN", serviceOn("STUN"), 112)}
                  {pill(884, 41, "TURN", serviceOn("TURN"), 112)}

                  <text x={72} y={75} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#64748b">/admin</text>
                  <text x={196} y={75} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#64748b">gRPC</text>
                  <text x={320} y={75} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#64748b">gRPC</text>
                  <text x={444} y={75} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#64748b">/ais</text>
                  <text x={568} y={75} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#64748b">/mfr</text>
                  <text x={692} y={75} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#64748b">/signaling</text>
                  <text x={816} y={75} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#64748b">:{icePort}</text>
                  <text x={940} y={75} textAnchor="middle" fontSize={7} fontFamily="monospace" fill="#64748b">:{icePort}, 49152-65535</text>

                  {/* Cluster internal label under Control + Signer */}
                  <line x1={140} y1={93} x2={376} y2={93} stroke="#d97706" strokeWidth={0.6} opacity={0.5} />
                  <text x={258} y={101} textAnchor="middle" fontSize={7} fill="#92400e" opacity={0.6}>cluster internal service</text>
                </g>

                {/* ═══════════════════════════════════════════ */}
                {/* ROW 3: Infrastructure  y=232 h=200         */}
                {/* 3 columns: w=326, gap=21                   */}
                {/* ═══════════════════════════════════════════ */}

                {/* ── Global Config (3 layers stacked) ── */}
                <g transform="translate(20,232)" style={{ cursor: "pointer" }} onClick={() => navigate("/admin/config")} onMouseEnter={() => setHovered("config")} onMouseLeave={() => setHovered(null)}>
                  <rect width={326} height={200} rx={14} fill="#eef2ff" stroke={hovered === "config" ? "#6366f1" : "none"} strokeWidth={1.5} opacity={0.85} />
                  <text x={16} y={22} textAnchor="start" fontSize={11} fontWeight={800} fill="#3730a3">
                    Global Config
                  </text>
                  <text x={310} y={22} textAnchor="end" fontSize={8} fontFamily="monospace" fill="#6366f1">
                    platform/config
                  </text>

                  {/* Effective summary */}
                  <text x={163} y={48} textAnchor="middle" fontSize={8} fontWeight={600} fill="#3730a3">
                    effective = L2 + L1 + L0
                  </text>

                  {/* L2: Overrides (top — highest priority) */}
                  <rect x={16} y={58} width={294} height={34} rx={8} fill="none" stroke="#a78bfa" strokeWidth={0.8} />
                  {/* Lucide: database */}
                  <svg x={27} y={66} width={18} height={18} viewBox="0 0 24 24" fill="none" stroke="#a78bfa" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                    <ellipse cx={12} cy={5} rx={9} ry={3} />
                    <path d="M3 5V19A9 3 0 0 0 21 19V5" />
                    <path d="M3 12A9 3 0 0 0 21 12" />
                  </svg>
                  <text x={52} y={78} textAnchor="start" fontSize={9} fontWeight={700} fill="#334155">L2  Overrides</text>
                  <text x={288} y={78} textAnchor="end" fontSize={7} fill="#94a3b8">dynamic conf stored in SQLite</text>

                  {/* L1: config.toml */}
                  <rect x={16} y={102} width={294} height={34} rx={8} fill="none" stroke="#818cf8" strokeWidth={0.8} />
                  {/* Lucide: file-cog */}
                  <svg x={27} y={110} width={18} height={18} viewBox="0 0 24 24" fill="none" stroke="#818cf8" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                    <path d="M15 8a1 1 0 0 1-1-1V2a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8z" />
                    <path d="M20 8v12a2 2 0 0 1-2 2h-4.182" />
                    <path d="m3.305 19.53.923-.382" />
                    <path d="M4 10.592V4a2 2 0 0 1 2-2h8" />
                    <path d="m4.228 16.852-.924-.383" />
                    <path d="m5.852 15.228-.383-.923" />
                    <path d="m5.852 20.772-.383.924" />
                    <path d="m8.148 15.228.383-.923" />
                    <path d="m8.53 21.696-.382-.924" />
                    <path d="m9.773 16.852.922-.383" />
                    <path d="m9.773 19.148.922.383" />
                    <circle cx={7} cy={18} r={3} />
                  </svg>
                  <text x={52} y={122} textAnchor="start" fontSize={9} fontWeight={700} fill="#334155">L1  config.toml</text>
                  <text x={288} y={122} textAnchor="end" fontSize={7} fill="#94a3b8">hot-reload via SIGHUP</text>

                  {/* L0: Built-in defaults */}
                  <rect x={16} y={146} width={294} height={34} rx={8} fill="none" stroke="#cbd5e1" strokeWidth={0.8} />
                  {/* Lucide: binary */}
                  <svg x={27} y={154} width={18} height={18} viewBox="0 0 24 24" fill="none" stroke="#94a3b8" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                    <rect x={14} y={14} width={4} height={6} rx={2} />
                    <rect x={6} y={4} width={4} height={6} rx={2} />
                    <path d="M6 20h4" />
                    <path d="M14 10h4" />
                    <path d="M6 14h2v6" />
                    <path d="M14 4h2v6" />
                  </svg>
                  <text x={52} y={166} textAnchor="start" fontSize={9} fontWeight={700} fill="#334155">L0  Defaults</text>
                  <text x={288} y={166} textAnchor="end" fontSize={7} fill="#94a3b8">built into binary</text>
                </g>

                {/* ── Realm Data (organic ellipse layout) ── */}
                <g transform="translate(367,232)" style={{ cursor: "pointer" }} onClick={() => navigate("/admin/realms")} onMouseEnter={() => setHovered("realm")} onMouseLeave={() => setHovered(null)}>
                  <rect width={326} height={200} rx={14} fill="#ecfdf5" stroke={hovered === "realm" ? "#22c55e" : "none"} strokeWidth={1.5} opacity={0.85} />
                  <text x={16} y={22} textAnchor="start" fontSize={11} fontWeight={800} fill="#166534">
                    Realms, security domain
                  </text>
                  <text x={310} y={22} textAnchor="end" fontSize={8} fontFamily="monospace" fill="#22c55e">
                    platform/realm
                  </text>

                  <g transform="translate(0,10)">
                  {/* Connecting lines — center-to-center direction, drawn edge-to-edge */}
                  {/* ACL(134,64) → Realm(78,148): exit ACL ~(115,93), enter Realm ~(96,121) */}
                  <line x1={115} y1={93} x2={96} y2={121} stroke="#86efac" strokeWidth={1} />
                  <text x={94} y={110} fontSize={6.5} fill="#6ee7b7">realm_id</text>
                  {/* ACL(134,64) → ActorType(242,120): exit ACL ~(180,88), enter ActorType ~(197,96) */}
                  <line x1={180} y1={88} x2={197} y2={96} stroke="#86efac" strokeWidth={1} strokeDasharray="3,3" />

                  {/* ACL ellipse — upper center-left */}
                  <ellipse cx={134} cy={64} rx={74} ry={30} fill="#d1fae5" fillOpacity={0.4} stroke="#4ade80" strokeWidth={0.8} />
                  <text x={134} y={56} textAnchor="middle" fontSize={11} fontWeight={700} fill="#166534">ACL</text>
                  <text x={134} y={68} textAnchor="middle" fontSize={7} fill="#475569">Allow / Deny from:</text>
                  <text x={134} y={78} textAnchor="middle" fontSize={7} fill="#475569">[$realm-id] [$actr-type]</text>

                  {/* Realm ellipse — lower left */}
                  <ellipse cx={78} cy={148} rx={64} ry={28} fill="#dcfce7" fillOpacity={0.5} stroke="#22c55e" strokeWidth={0.8} />
                  <text x={78} y={140} textAnchor="middle" fontSize={11} fontWeight={700} fill="#166534">Realm</text>
                  <text x={78} y={152} textAnchor="middle" fontSize={7} fill="#475569">id · name · status · enabled</text>
                  <text x={78} y={162} textAnchor="middle" fontSize={7} fill="#475569">secret · expires</text>

                  {/* ActorType ellipse — right, higher than Realm */}
                  <ellipse cx={242} cy={120} rx={76} ry={32} fill="#bbf7d0" fillOpacity={0.35} stroke="#86efac" strokeWidth={0.8} />
                  <text x={242} y={112} textAnchor="middle" fontSize={11} fontWeight={700} fill="#166534">ActorType</text>
                  <text x={242} y={124} textAnchor="middle" fontSize={7} fill="#475569">manufacturer : name : version</text>
                  <text x={242} y={134} textAnchor="middle" fontSize={7} fill="#475569">ServiceSpec · fingerprint</text>
                  </g>

                </g>

                {/* ── Recording Pipeline (vertical flow) ── */}
                <g transform="translate(714,232)" style={{ cursor: "pointer" }} onClick={() => navigate("/admin/general/recording")} onMouseEnter={() => setHovered("recording")} onMouseLeave={() => setHovered(null)}>
                  <rect width={326} height={200} rx={14} fill="#fdf2f8" stroke={hovered === "recording" ? "#db2777" : "none"} strokeWidth={1.5} opacity={0.85} />
                  <text x={16} y={22} textAnchor="start" fontSize={11} fontWeight={800} fill="#9d174d">
                    Recording Pipeline
                  </text>
                  <text x={310} y={22} textAnchor="end" fontSize={8} fontFamily="monospace" fill="#db2777">
                    platform/recording
                  </text>

                  {/* Pipeline description */}
                  <text x={163} y={48} textAnchor="middle" fontSize={8} fontWeight={600} fill="#9d174d" opacity={0.6}>
                    channel: filter → route → sink
                  </text>

                  {/* Channel pills row */}
                  {(() => {
                    const gap = 6;
                    const charCounts = channelData.map((ch) => ch.label.length);
                    const totalChars = charCounts.reduce((a, b) => a + b, 0);
                    const availW = 294 - (channelData.length - 1) * gap;
                    const widths = charCounts.map((n) => Math.round(availW * n / totalChars));
                    const totalW = widths.reduce((a, b) => a + b, 0) + (widths.length - 1) * gap;
                    const startX = 16 + (294 - totalW) / 2;
                    let accX = startX;
                    return channelData.map((ch, i) => {
                    const cw = widths[i];
                    const x = accX;
                    accX += cw + gap;
                    const cx = x + cw / 2;
                    return (
                      <g key={ch.id}>
                        <rect x={x} y={58} width={cw} height={34} rx={6} fill={ch.bg} stroke={ch.color} strokeWidth={0.8} />
                        <text x={cx} y={75} textAnchor="middle" fontSize={8} fontWeight={700} fill={ch.color} dominantBaseline="central">
                          {ch.label}
                        </text>
                      </g>
                    );
                  }); })()}


                  {/* Sink targets row + connecting lines */}
                  {(() => {
                    const sinkStyles: Record<string, { label: string }> = {
                      file:   { label: "File" },
                      otlp:   { label: "OTLP" },
                      stderr: { label: "stderr" },
                    };
                    const usedTypes = [...new Set(channelData.map((c) => c.sinkType))];
                    const snkGap = 10;
                    const snkW = Math.min(180, Math.max(50, (294 - (usedTypes.length - 1) * snkGap) / usedTypes.length));
                    const totalW = usedTypes.length * snkW + (usedTypes.length - 1) * snkGap;
                    const snkLeft = (294 - totalW) / 2 + 16;
                    const snkY = 146;

                    // Compute sink center-x positions by type
                    const sinkCenters: Record<string, number> = {};
                    usedTypes.forEach((type, i) => {
                      sinkCenters[type] = snkLeft + i * (snkW + snkGap) + snkW / 2;
                    });

                    // Channel pill geometry (must match pill widths above)
                    const chGap = 6;
                    const chCharCounts = channelData.map((ch) => ch.label.length);
                    const chTotalChars = chCharCounts.reduce((a, b) => a + b, 0);
                    const chAvailW = 294 - (channelData.length - 1) * chGap;
                    const chWidths = chCharCounts.map((n) => Math.round(chAvailW * n / chTotalChars));
                    const chTotalW = chWidths.reduce((a, b) => a + b, 0) + (chWidths.length - 1) * chGap;
                    const chStartX = 16 + (294 - chTotalW) / 2;
                    const chBottom = 92; // y=58 + h=34

                    // Draw connecting curves (channel bottom → sink top, gentle arc)
                    const midY = (chBottom + snkY) / 2;
                    const lines = channelData.map((ch, i) => {
                      let cx = chStartX;
                      for (let j = 0; j < i; j++) cx += chWidths[j] + chGap;
                      cx += chWidths[i] / 2;
                      const tx = sinkCenters[ch.sinkType];
                      if (tx === undefined) return null;
                      return (
                        <path
                          key={`line-${ch.id}`}
                          d={`M${cx},${chBottom + 2} C${cx},${midY} ${tx},${midY} ${tx},${snkY - 2}`}
                          fill="none"
                          stroke={ch.color}
                          strokeWidth={0.8}
                          opacity={0.45}
                        />
                      );
                    });

                    const sinkBoxes = usedTypes.map((type, i) => {
                      const st = sinkStyles[type] ?? sinkStyles.stderr;
                      const x = snkLeft + i * (snkW + snkGap);
                      const cx = x + snkW / 2;
                      const uris = [...new Set(
                        channelData.filter((c) => c.sinkType === type).map((c) => c.sink),
                      )];
                      return (
                        <g key={type}>
                          <rect x={x} y={snkY} width={snkW} height={34} rx={8} fill="none" stroke="#94a3b8" strokeWidth={0.8} />
                          <text x={cx} y={snkY + 14} textAnchor="middle" fontSize={9} fontWeight={700} fill="#475569">
                            {st.label}
                          </text>
                          {uris.map((u, j) => (
                            <text key={j} x={cx} y={snkY + 26 + j * 10} textAnchor="middle" fontSize={6.5} fontFamily="monospace" fill="#64748b" opacity={0.7}>
                              {short(u, 22)}
                            </text>
                          ))}
                        </g>
                      );
                    });

                    return [...lines, ...sinkBoxes];
                  })()}

                </g>

              </svg>
            </div>

            <ConfigSection
              storageKey="arch"
              groups={nodeSections.map((s) => ({
                title: s.title,
                description: s.desc,
                fields: filterFields(d.config_fields, s.keys),
              }))}
              onRefresh={fetchData}
            />
          </ServicePageLayout>
        );
      }}
    </PlatformDataGuard>
  );
}
