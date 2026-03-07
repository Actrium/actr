import { usePlatformData, filterFields, PlatformDataGuard } from "./shared";
import { ServicePageLayout, ConfigSection } from "../../components/layout/ServicePageLayout";
import { HowItWorks } from "../../components/ui/HowItWorks";
import type { ResolvedField } from "../../lib/api";

/* ── helpers ─────────────────────────────────────────────────── */

function fieldValue(fields: ResolvedField[], key: string): string | null {
  const f = fields.find((f) => f.key === key);
  if (!f) return null;
  const v = f.effective_value.trim();
  if (!v || v === "none" || v === "None") return null;
  return v;
}

function sinkType(uri: string | null): "file" | "otlp" | "stderr" {
  if (!uri) return "stderr";
  if (uri.startsWith("file://")) return "file";
  if (uri.startsWith("otlp+")) return "otlp";
  return "stderr";
}

function shortUri(uri: string | null, maxLen = 36): string {
  if (!uri) return "stderr";
  if (uri.length <= maxLen) return uri;
  return uri.slice(0, maxLen - 1) + "\u2026";
}

/* ── channel palette ─────────────────────────────────────────── */

const CHANNELS = [
  {
    id: "observability",
    label: "Observability",
    sub: "Latency \u00b7 health \u00b7 metrics",
    fill: "#dbeafe",
    stroke: "#3b82f6",
    text: "#1e40af",
  },
  {
    id: "audit",
    label: "Audit",
    sub: "Config changes \u00b7 access",
    fill: "#fef3c7",
    stroke: "#d97706",
    text: "#92400e",
  },
  {
    id: "security",
    label: "Security",
    sub: "Auth failures \u00b7 abuse",
    fill: "#fce7f3",
    stroke: "#db2777",
    text: "#9d174d",
  },
  {
    id: "operations",
    label: "Operations",
    sub: "Restarts \u00b7 deployments",
    fill: "#e0e7ff",
    stroke: "#6366f1",
    text: "#3730a3",
  },
] as const;

/* ── Sink target styles ──────────────────────────────────────── */

const SINK_STYLE = {
  file:   { fill: "#ecfdf5", stroke: "#059669", text: "#065f46", label: "File Sink",   lineColor: "#059669" },
  otlp:   { fill: "#ede9fe", stroke: "#7c3aed", text: "#4c1d95", label: "OTLP Sink",   lineColor: "#7c3aed" },
  stderr: { fill: "#f3f4f6", stroke: "#9ca3af", text: "#374151", label: "stderr",       lineColor: "#9ca3af" },
} as const;

type SinkKind = keyof typeof SINK_STYLE;

/* ── The diagram ─────────────────────────────────────────────── */

function RecordingDiagram({ fields }: { fields: ResolvedField[] }) {
  const globalSink = fieldValue(fields, "recording.sink");

  /* Resolve each channel */
  const resolved = CHANNELS.map((ch) => {
    const perChannel = fieldValue(fields, `recording.${ch.id}.sink`);
    const uri = perChannel ?? globalSink;
    const type = sinkType(uri);
    const isOverride = perChannel !== null;
    const filter = fieldValue(fields, `recording.${ch.id}.filter`) ?? "—";
    return { ...ch, uri, type, isOverride, filter };
  });

  const usedTypes = [...new Set(resolved.map((r) => r.type))] as SinkKind[];

  /* ── Layout ── */
  const W = 620;
  const chW = 128;
  const chH = 50;
  const chGap = 16;
  const N = CHANNELS.length;
  const totalChW = N * chW + (N - 1) * chGap;
  const chLeft = (W - totalChW) / 2;

  const ingestTop = 6;
  const chY = 34;
  const routeTop = chY + chH;
  const sinkY = routeTop + 56;
  const snkH = 56;
  const H = sinkY + snkH + 8;

  const snkW = 200;
  const snkGap = 24;
  const totalSnkW = usedTypes.length * snkW + (usedTypes.length - 1) * snkGap;
  const snkLeft = (W - totalSnkW) / 2;

  const chCx = (i: number) => chLeft + i * (chW + chGap) + chW / 2;
  const snkCx = (i: number) => snkLeft + i * (snkW + snkGap) + snkW / 2;

  const sinkCxMap: Record<string, number> = {};
  usedTypes.forEach((t, i) => { sinkCxMap[t] = snkCx(i); });

  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      className="max-w-3xl mx-auto"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <marker id="rd-gray" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#94a3b8" />
        </marker>
        <marker id="rd-file" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#059669" />
        </marker>
        <marker id="rd-otlp" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#7c3aed" />
        </marker>
        <marker id="rd-stderr" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#9ca3af" />
        </marker>
      </defs>

      {/* ══ Ingest arrows (↓ into channels) ══ */}
      {CHANNELS.map((_, i) => (
        <line key={`in-${i}`}
          x1={chCx(i)} y1={ingestTop} x2={chCx(i)} y2={chY - 3}
          stroke="#94a3b8" strokeWidth="1.2" markerEnd="url(#rd-gray)"
        />
      ))}

      {/* ══ Channel boxes ══ */}
      {resolved.map((ch, i) => {
        const x = chLeft + i * (chW + chGap);
        const cx = chCx(i);
        return (
          <g key={ch.id}>
            <rect
              x={x} y={chY} width={chW} height={chH} rx="8"
              fill={ch.fill} stroke={ch.stroke} strokeWidth="1.2"
            />
            <text x={cx} y={chY + 17} textAnchor="middle"
              fontSize="10" fontWeight="700" fill={ch.text}>{ch.label}</text>
            <text x={cx} y={chY + 29} textAnchor="middle"
              fontSize="7" fill={ch.text} opacity="0.7">{ch.sub}</text>
            <text x={cx} y={chY + 41} textAnchor="middle"
              fontSize="7.5" fontFamily="monospace" fill={ch.text} opacity="0.55">
              filter:{ch.filter}
            </text>

            {ch.isOverride && (
              <text x={x + chW - 7} y={chY + 11} textAnchor="middle"
                fontSize="7" fill={ch.text} opacity="0.5" fontWeight="600">{"\u25cf"}</text>
            )}
          </g>
        );
      })}

      {/* ══ Routing curves: channel → sink ══ */}
      {resolved.map((ch, i) => {
        const fromX = chCx(i);
        const fromY = chY + chH + 2;
        const toX = sinkCxMap[ch.type];
        const toY = sinkY - 3;
        const st = SINK_STYLE[ch.type];
        const midY = (fromY + toY) / 2;

        return (
          <path key={`route-${ch.id}`}
            d={`M${fromX},${fromY} C${fromX},${midY} ${toX},${midY} ${toX},${toY}`}
            fill="none" stroke={st.lineColor} strokeWidth="1.2"
            opacity="0.6"
          />
        );
      })}

      {/* ══ Sink target boxes ══ */}
      {usedTypes.map((type, i) => {
        const st = SINK_STYLE[type];
        const x = snkLeft + i * (snkW + snkGap);
        const cx = snkCx(i);
        const uris = [...new Set(
          resolved.filter((ch) => ch.type === type).map((ch) => ch.uri ?? "stderr"),
        )];

        return (
          <g key={type}>
            <rect
              x={x} y={sinkY} width={snkW} height={snkH} rx="10"
              fill={st.fill} stroke={st.stroke} strokeWidth="1.5"
            />
            <text x={cx} y={sinkY + 18} textAnchor="middle"
              fontSize="10" fontWeight="700" fill={st.text}>{st.label}</text>
            {uris.map((u, j) => (
              <text key={j} x={cx} y={sinkY + 33 + j * 12} textAnchor="middle"
                fontSize="7" fontFamily="monospace" fill={st.text} opacity="0.7">
                {shortUri(u)}
              </text>
            ))}
          </g>
        );
      })}
    </svg>
  );
}

/* ── Page component ──────────────────────────────────────────── */

export function RecordingSection() {
  const { data, error, fetchData } = usePlatformData();

  return (
    <PlatformDataGuard data={data} error={error}>
      {(d) => {
        const recFields = filterFields(d.config_fields, ["recording"]);
        return (
          <ServicePageLayout
            title="Recording"
            description="Structured logging with four semantic channels, each with its own filter vocabulary and sink routing"
          >
            <HowItWorks storageKey="recording">
              <RecordingDiagram fields={recFields} />
              <div className="space-y-2 text-xs text-gray-500">
                <p className="font-semibold text-gray-600">The four channels</p>
                <ul className="list-disc pl-4 space-y-1.5">
                  <li>
                    <strong className="text-gray-600">Observability</strong> — runtime performance
                    and component health. Filter controls resolution: <code className="text-[11px] bg-gray-100 px-1 rounded">digest</code> (health summaries),{" "}
                    <code className="text-[11px] bg-gray-100 px-1 rounded">detailed</code> (+ per-request metrics),{" "}
                    <code className="text-[11px] bg-gray-100 px-1 rounded">full</code> (+ internal snapshots).
                    Default: <code className="text-[11px] bg-gray-100 px-1 rounded">digest</code>
                  </li>
                  <li>
                    <strong className="text-gray-600">Audit</strong> — who did what and when.
                    Filter controls scope: <code className="text-[11px] bg-gray-100 px-1 rounded">mutations</code> (write actions only),{" "}
                    <code className="text-[11px] bg-gray-100 px-1 rounded">all</code> (+ reads, queries).
                    Default: <code className="text-[11px] bg-gray-100 px-1 rounded">mutations</code>
                  </li>
                  <li>
                    <strong className="text-gray-600">Security</strong> — policy enforcement and
                    threat signals. Filter controls severity threshold:{" "}
                    <code className="text-[11px] bg-gray-100 px-1 rounded">critical</code> /{" "}
                    <code className="text-[11px] bg-gray-100 px-1 rounded">high</code> /{" "}
                    <code className="text-[11px] bg-gray-100 px-1 rounded">medium</code> /{" "}
                    <code className="text-[11px] bg-gray-100 px-1 rounded">all</code>.
                    Default: <code className="text-[11px] bg-gray-100 px-1 rounded">all</code>
                  </li>
                  <li>
                    <strong className="text-gray-600">Operations</strong> — infrastructure lifecycle.
                    Filter controls detail: <code className="text-[11px] bg-gray-100 px-1 rounded">lifecycle</code> (start/stop, deploy, reload),{" "}
                    <code className="text-[11px] bg-gray-100 px-1 rounded">detailed</code> (+ migration steps, state transitions).
                    Default: <code className="text-[11px] bg-gray-100 px-1 rounded">lifecycle</code>
                  </li>
                </ul>
              </div>

              <div className="mt-4 space-y-2 text-xs text-gray-500 border-t border-gray-100 pt-4">
                <p className="font-semibold text-gray-600">Physical sinks</p>
                <ul className="list-disc pl-4 space-y-1.5">
                  <li>
                    <strong className="text-gray-600">file://</strong> — writes structured JSON lines
                    to a local path. Non-blocking async I/O; no ANSI escape codes. Suitable for
                    logrotate + shipping via Filebeat / Fluentd.
                  </li>
                  <li>
                    <strong className="text-gray-600">otlp+http:// / otlp+grpc://</strong> — exports
                    records as OpenTelemetry spans to a remote collector. HTTP uses protobuf over
                    HTTP/1.1; gRPC uses tonic over HTTP/2. Both support batching and retry.
                  </li>
                </ul>
              </div>

              <div className="mt-4 space-y-2 text-xs text-gray-500 border-t border-gray-100 pt-4">
                <p className="font-semibold text-gray-600">Configuration</p>
                <ul className="list-disc pl-4 space-y-1.5">
                  <li>
                    <strong className="text-gray-600">recording.sink</strong> — global default sink for all
                    channels. If omitted, output goes to stderr.
                  </li>
                  <li>
                    <strong className="text-gray-600">recording.&lt;channel&gt;.filter</strong> — per-channel
                    semantic filter. Each channel has its own vocabulary (e.g. digest/detailed/full
                    for observability, mutations/all for audit).
                  </li>
                  <li>
                    <strong className="text-gray-600">recording.&lt;channel&gt;.sink</strong> — per-channel
                    sink override. Takes precedence over the global sink for that channel.
                  </li>
                  <li>
                    <strong className="text-gray-600">RUST_LOG</strong> — env var overrides all
                    per-channel levels when set.
                  </li>
                </ul>
              </div>
            </HowItWorks>

            <ConfigSection storageKey="recording" fields={recFields} onRefresh={fetchData} />
          </ServicePageLayout>
        );
      }}
    </PlatformDataGuard>
  );
}
