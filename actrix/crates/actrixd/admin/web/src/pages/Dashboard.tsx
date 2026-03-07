import { useEffect, useState, useCallback, useRef, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { AreaChart, Area, ResponsiveContainer } from "recharts";
import { api, type NodeInfo, type PlatformDetail, type MetricSample } from "../lib/api";
import { PowerReserveGauge } from "../components/dashboard/PowerReserveGauge";
import { ServicePageLayout } from "../components/layout/ServicePageLayout";
import { formatUptime } from "../lib/utils";

/* ── Click-to-copy value ───────────────────────────────── */

function Copyable({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    if (!value || value === "—") return;
    const write = navigator.clipboard?.writeText
      ? (t: string) => navigator.clipboard.writeText(t)
      : (t: string) => {
          const ta = document.createElement("textarea");
          ta.value = t;
          ta.style.position = "fixed";
          ta.style.opacity = "0";
          document.body.appendChild(ta);
          ta.select();
          document.execCommand("copy");
          document.body.removeChild(ta);
          return Promise.resolve();
        };
    write(value).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    });
  };

  const handleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    copy();
  };

  return (
    <span
      onClick={handleClick}
      className="cursor-pointer hover:text-blue-600 transition-colors relative"
      title="Click to copy"
    >
      {value}
      {copied && <span className="absolute bottom-0 text-xs text-green-500 ml-1">copied</span>}
    </span>
  );
}

/* ── Formatting helpers ───────────────────────────────── */

function fmtNum(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1).replace(/\.0$/, "") + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1).replace(/\.0$/, "") + "k";
  return String(n);
}

/* ── Service bitmask ───────────────────────────────────── */

const SERVICES = [
  { name: "Signaling", bit: 1, type: 3, route: "/admin/services/signaling" },
  { name: "STUN", bit: 2, type: 1, route: "/admin/services/stun" },
  { name: "TURN", bit: 4, type: 2, route: "/admin/services/turn" },
  { name: "AIS", bit: 8, type: 4, route: "/admin/services/ais" },
  { name: "KS", bit: 16, type: 5, route: "/admin/services/ks" },
] as const;

/* ── Helpers for timeseries data ────────────────────────── */

/** Extract sparkline data from real metric samples */
function samplesToSpark(samples: MetricSample[], key: keyof MetricSample): { v: number }[] {
  if (samples.length === 0) return [];
  return samples.map((s) => ({ v: s[key] as number }));
}

/** Get the latest value from samples, or 0 */
function latestVal(samples: MetricSample[], key: keyof MetricSample): number {
  if (samples.length === 0) return 0;
  return samples[samples.length - 1][key] as number;
}

function SparkCell({ data, color, value, label }: { data: { v: number }[]; color: string; value: string; label: string }) {
  return (
    <div className="pointer-events-none">
      <p className="text-xs text-gray-400 leading-none">{label}</p>
      <p className="text-lg font-bold tabular-nums leading-none mt-2" style={{ color }}>{value}</p>
      <div className="mt-1.5">
        <ResponsiveContainer width="100%" height={28}>
          <AreaChart data={data} margin={{ top: 0, right: 0, bottom: 0, left: 0 }}>
            <Area type="monotone" dataKey="v" stroke={color} strokeWidth={1.5} fill="none" dot={false} isAnimationActive={false} />
          </AreaChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}

function MicroCharts({ samples }: { samples: MetricSample[] }) {
  const conns = latestVal(samples, "active_conns");
  const reqs = latestVal(samples, "requests");
  const fails = latestVal(samples, "failed_requests");
  const p95 = latestVal(samples, "latency_p95_ms");
  const successRate = reqs > 0 ? ((reqs - fails) / reqs) * 100 : 100;

  const data = useMemo(() => ({
    conns: samplesToSpark(samples, "active_conns"),
    p95: samplesToSpark(samples, "latency_p95_ms"),
    reqs: samplesToSpark(samples, "requests"),
    success: samples.map((s) => {
      const r = s.requests;
      return { v: r > 0 ? ((r - s.failed_requests) / r) * 100 : 100 };
    }),
  }), [samples]);

  return (
    <div className="grid grid-cols-2 gap-x-4 gap-y-3 pointer-events-none">
      <SparkCell data={data.conns} color="#1a3a6a" value={fmtNum(conns)} label="active conns" />
      <SparkCell data={data.success} color={successRate >= 99 ? "#22c55e" : successRate >= 95 ? "#e87a20" : "#dc2626"} value={(successRate % 1 === 0 ? successRate.toFixed(0) : successRate.toFixed(1)) + "%"} label="success rate" />
      <SparkCell data={data.p95} color={p95 <= 5 ? "#22c55e" : p95 <= 20 ? "#e87a20" : "#dc2626"} value={p95.toFixed(1) + "ms"} label="p95 latency" />
      <SparkCell data={data.reqs} color="#1a3a6a" value={fmtNum(reqs)} label="requests" />
    </div>
  );
}

/* ── Dashboard ─────────────────────────────────────────── */

export function Dashboard() {
  const navigate = useNavigate();
  const [info, setInfo] = useState<NodeInfo | null>(null);
  const [platform, setPlatform] = useState<PlatformDetail | null>(null);
  const [error, setError] = useState("");
  const [timeseries, setTimeseries] = useState<Record<number, MetricSample[]>>({});

  /* Gauge animation */
  const [gaugeValue, setGaugeValue] = useState(0);
  const gaugeTargetRef = useRef(0);
  const gaugeValueRef = useRef(0);
  const animatingRef = useRef(false);
  const isMountedRef = useRef(true);

  useEffect(() => {
    isMountedRef.current = true;
    return () => { isMountedRef.current = false; };
  }, []);

  useEffect(() => { gaugeValueRef.current = gaugeValue; }, [gaugeValue]);

  const setGauge = useCallback((v: number) => {
    gaugeValueRef.current = v;
    setGaugeValue(v);
  }, []);

  const fetchData = useCallback(async () => {
    try {
      const [nodeData, platformData] = await Promise.all([
        api.getNodeInfo(),
        api.getPlatformDetail().catch(() => null),
      ]);
      if (isMountedRef.current) {
        setInfo(nodeData);
        if (platformData) setPlatform(platformData);
        setError("");
      }

      // Fetch tier 0 timeseries for all service types in parallel
      const svcTypes = [1, 2, 3, 4, 5];
      const tsResults = await Promise.all(
        svcTypes.map((t) => api.getMetricsTimeseries(t, 0).catch(() => null)),
      );
      if (isMountedRef.current) {
        const ts: Record<number, MetricSample[]> = {};
        svcTypes.forEach((t, i) => {
          if (tsResults[i]?.samples) ts[t] = tsResults[i]!.samples;
        });
        setTimeseries(ts);
      }
    } catch (err) {
      if (isMountedRef.current) {
        setError(err instanceof Error ? err.message : "Failed to load");
      }
    }
  }, []);

  useEffect(() => {
    if (!info) return;
    const target = info.power_reserve;
    gaugeTargetRef.current = target;
    const current = gaugeValueRef.current;

    if (current === 0 && target > 0 && !animatingRef.current) {
      animatingRef.current = true;
      const from = Math.min(target * 1.02, 5);
      const start = Date.now();
      const animate = () => {
        const p = Math.min((Date.now() - start) / 500, 1);
        const t = -(Math.cos(Math.PI * p) - 1) / 2;
        setGauge(from + (gaugeTargetRef.current - from) * t);
        if (p < 1) requestAnimationFrame(animate);
        else animatingRef.current = false;
      };
      requestAnimationFrame(animate);
    } else if (!animatingRef.current && Math.abs(current - target) > 0.05) {
      animatingRef.current = true;
      const from = current;
      const start = Date.now();
      const animate = () => {
        const p = Math.min((Date.now() - start) / 2000, 1);
        const t = -(Math.cos(Math.PI * p) - 1) / 2;
        setGauge(from + (gaugeTargetRef.current - from) * t);
        if (p < 1) requestAnimationFrame(animate);
        else animatingRef.current = false;
      };
      requestAnimationFrame(animate);
    }
  }, [info, setGauge]);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  /* ── Helpers ── */

  const enableMask = (() => {
    if (!platform) return 0;
    const f = platform.config_fields.find((x) => x.key === "enable");
    return f ? parseInt(f.effective_value, 10) || 0 : 0;
  })();

  const fieldVal = (key: string) => {
    const f = platform?.config_fields.find((x) => x.key === key);
    return f?.effective_value?.trim() || "—";
  };


  /* ── Render ── */

  if (error && !info) {
    return (
      <div className="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-700">{error}</div>
    );
  }
  if (!info) {
    return <div className="text-sm text-gray-500">Loading...</div>;
  }

  const allHealthy = info.services.every((s) => s.is_healthy);

  /* Host info from platform config */
  const httpPort = fieldVal("bind.http.port");
  const icePort = fieldVal("bind.ice.port");
  const advIp = fieldVal("bind.ice.advertised_ip");
  const relayRange = fieldVal("turn.relay_port_range");
  const hostname = typeof window !== "undefined" ? window.location.hostname : "—";

  return (
    <ServicePageLayout
      title="Dashboard"
      description="At-a-glance overview"
      headerActions={
        <span
          className={
            allHealthy
              ? "inline-flex items-center rounded-full bg-green-50 px-3 py-1 text-xs font-medium text-green-700"
              : "inline-flex items-center rounded-full bg-red-50 px-3 py-1 text-xs font-medium text-red-700"
          }
        >
          {allHealthy ? "All systems operational" : "Degraded"}
        </span>
      }
    >
      {/* ═══ Services ═══ */}
      <div>
        <h2 className="mb-3 text-xs font-bold text-gray-400 uppercase tracking-wide">Services</h2>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {SERVICES.map((svc) => {
            const enabled = (enableMask & svc.bit) !== 0;
            const status = info.services.find((s) => s.type === svc.type);
            const healthy = status?.is_healthy ?? false;
            const samples = timeseries[svc.type] ?? [];

            return (
              <div
                key={svc.name}
                onClick={() => navigate(svc.route)}
                className="rounded-xl border border-gray-200 bg-transparent p-5 cursor-pointer hover:ring-2 hover:ring-blue-400 transition-all"
              >
                {/* Header: name + dot + 15min badge */}
                <div className="flex items-center gap-3 mb-3">
                  <span
                    className={`relative flex h-3 w-3 items-center justify-center rounded-full border ${
                      !enabled
                        ? "border-gray-300"
                        : healthy
                          ? "border-green-400"
                          : "border-red-400"
                    }`}
                  >
                    <span className={`h-1.5 w-1.5 rounded-full ${
                      !enabled ? "bg-gray-300" : healthy ? "bg-green-500" : "bg-red-500"
                    }`} />
                    {enabled && healthy && (
                      <span className="absolute inset-0 rounded-full border border-green-400 animate-ping opacity-40" />
                    )}
                  </span>
                  <span className="text-xs font-medium text-gray-500">{svc.name}</span>
                  {enabled && status && (
                    <span className="ml-auto text-xs text-gray-400">15min</span>
                  )}
                </div>

                {enabled && status ? (
                  <MicroCharts samples={samples} />
                ) : (
                  <div className="py-6 text-center">
                    <p className="text-lg font-medium text-gray-300">Off</p>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* ═══ Node ═══ */}
      <div>
        <h2 className="mb-3 text-xs font-bold text-gray-400 uppercase tracking-wide">Node</h2>
        <div
          className="flex flex-col lg:flex-row gap-6 items-center rounded-xl border border-gray-200 p-5 cursor-pointer hover:ring-2 hover:ring-blue-400 transition-all"
          onClick={() => navigate("/admin/host/status")}
        >
          {/* Gauge */}
          <div className="flex justify-start -ml-4">
            <PowerReserveGauge value={gaugeValue} size={220} />
          </div>

          {/* Info table */}
          <div className="space-y-3">
            {/* Row 1: host */}
            <div className="flex gap-14">
              <div className="whitespace-nowrap">
                <p className="text-xs text-gray-400">hostname</p>
                <p className="text-sm font-mono text-gray-900"><Copyable value={hostname} /></p>
              </div>
              <div className="whitespace-nowrap">
                <p className="text-xs text-gray-400">advertised ip</p>
                <p className="text-sm font-mono text-gray-900"><Copyable value={advIp} /></p>
              </div>
            </div>
            {/* Row 2: ports */}
            <div className="flex gap-14">
              {[
                { label: "http", value: `${httpPort}/tcp`, healthy: info.services.some(s => s.is_healthy) },
                { label: "ice", value: `${icePort}/udp`, healthy: info.services.some(s => (s.type === 1 || s.type === 2) && s.is_healthy) },
                { label: "relay", value: `${relayRange}/udp`, healthy: info.services.some(s => s.type === 2 && s.is_healthy) },
              ].map((port) => (
                <div key={port.label} className="whitespace-nowrap">
                  <p className="text-xs text-gray-400">{port.label}</p>
                  <p className="text-sm font-mono text-gray-900 flex items-center gap-2">
                    <span className="relative flex">
                      <span className={`h-2 w-2 rounded-full ${port.healthy ? "bg-green-500" : "bg-red-500"}`} />
                      {port.healthy && <span className="absolute inset-0 h-2 w-2 rounded-full bg-green-500 animate-ping opacity-40" />}
                    </span>
                    <Copyable value={port.value} />
                  </p>
                </div>
              ))}
            </div>
            {/* Row 3: identity */}
            <div className="flex gap-14">
              <div className="whitespace-nowrap">
                <p className="text-xs text-gray-400">node id</p>
                <p className="text-sm font-mono text-gray-900"><Copyable value={info.node_id} /></p>
              </div>
              <div className="whitespace-nowrap">
                <p className="text-xs text-gray-400">name</p>
                <p className="text-sm text-gray-900"><Copyable value={info.name} /></p>
              </div>
              <div className="whitespace-nowrap">
                <p className="text-xs text-gray-400">version</p>
                <p className="text-sm font-mono text-gray-900"><Copyable value={info.version} /></p>
              </div>
              <div className="whitespace-nowrap">
                <p className="text-xs text-gray-400">uptime</p>
                <p className="text-sm text-gray-900"><Copyable value={formatUptime(info.uptime_secs)} /></p>
              </div>
              <div className="whitespace-nowrap">
                <p className="text-xs text-gray-400">location</p>
                <p className="text-sm text-gray-900"><Copyable value={info.location_tag} /></p>
              </div>
            </div>
          </div>
        </div>
      </div>

    </ServicePageLayout>
  );
}
