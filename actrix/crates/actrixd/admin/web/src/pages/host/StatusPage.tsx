import { useEffect, useState, useCallback, useRef } from "react";
import { api, type NodeInfo, type MetricDetail } from "../../lib/api";
import { PowerReserveGauge } from "../../components/dashboard/PowerReserveGauge";
import { ServicePageLayout } from "../../components/layout/ServicePageLayout";
import { CollapsibleCard } from "../../components/ui/CollapsibleCard";

/** Metric display metadata */
interface MetricMeta {
  label: string;
  description: string;
  /** Format the raw value for display. Default: percentage */
  format?: (v: number) => string;
}

const METRIC_META: Record<string, MetricMeta> = {
  cpu_usage: {
    label: "CPU Usage",
    description: "Processor utilization across all cores",
  },
  cpu_io_wait: {
    label: "CPU I/O Wait",
    description: "Time the CPU spends waiting for disk or network I/O to complete",
  },
  cpu_load: {
    label: "CPU Load",
    description: "System load average relative to available cores; values above 1.0 indicate overload",
    format: (v) => v.toFixed(2),
  },
  memory_usage: {
    label: "Memory",
    description: "Physical RAM utilization",
  },
  memory_pressure: {
    label: "Memory Pressure",
    description: "Swap activity and page-reclaim pressure on the memory subsystem",
  },
  disk_io_utilization: {
    label: "Disk I/O",
    description: "Storage device utilization from /proc/diskstats",
  },
  network_dropped_packets: {
    label: "Network Drops",
    description: "Ratio of dropped packets across all network interfaces",
  },
  file_descriptors: {
    label: "File Descriptors",
    description: "Open file descriptors relative to system-wide limit (fs.file-max)",
  },
  process_count: {
    label: "Processes",
    description: "Running process count relative to kernel limit (pid_max)",
  },
};

/** Canonical display order */
const METRIC_ORDER = [
  "cpu_usage",
  "cpu_io_wait",
  "cpu_load",
  "memory_usage",
  "memory_pressure",
  "disk_io_utilization",
  "network_dropped_packets",
  "file_descriptors",
  "process_count",
];

function formatPercent(v: number): string {
  return `${(v * 100).toFixed(1)}%`;
}

function scoreColor(score: number): string {
  if (score >= 4) return "text-green-600";
  if (score >= 3) return "text-yellow-600";
  if (score >= 2) return "text-orange-500";
  return "text-red-600";
}

function scoreBg(score: number): string {
  if (score >= 4) return "bg-green-50";
  if (score >= 3) return "bg-yellow-50";
  if (score >= 2) return "bg-orange-50";
  return "bg-red-50";
}

/** A value cell that briefly flashes when its displayed text changes. */
function V({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  const text = String(Array.isArray(children) ? children.join("") : children ?? "");
  const prev = useRef(text);
  const [flash, setFlash] = useState(false);

  useEffect(() => {
    if (text !== prev.current) {
      prev.current = text;
      setFlash(true);
      const t = setTimeout(() => setFlash(false), 600);
      return () => clearTimeout(t);
    }
  }, [text]);

  return (
    <span className={className}>
      <span className={flash ? "transition-colors duration-300 text-blue-500" : "transition-colors duration-300 inherit"}>
        {children}
      </span>
    </span>
  );
}

function MetricRow({ id, detail }: { id: string; detail: MetricDetail }) {
  const meta = METRIC_META[id];
  if (!meta) return null;

  const fmt = meta.format ?? formatPercent;

  return (
    <tr className="border-b border-gray-100 last:border-0 align-top">
      <td className="py-2 pr-4 font-medium text-gray-900 whitespace-nowrap">{meta.label}</td>
      <td className="py-2 pr-4 text-xs text-gray-400">
        <span className="hidden lg:inline">{meta.description}</span>
        <span className="lg:hidden relative group cursor-help inline-flex items-center" title={meta.description}>
          <svg className="w-3.5 h-3.5 text-gray-300" viewBox="0 0 16 16" fill="currentColor">
            <path d="M8 1a7 7 0 100 14A7 7 0 008 1zm0 2.5a1 1 0 110 2 1 1 0 010-2zM6.5 7h2v4.5h-2V7z" />
          </svg>
          <span className="invisible group-hover:visible absolute left-5 -top-1 z-10 w-52 rounded-md bg-gray-800 px-2.5 py-1.5 text-[11px] leading-snug text-gray-100 shadow-lg">
            {meta.description}
          </span>
        </span>
      </td>
      <td className="py-2 pr-4 text-right">
        <V className="font-medium text-gray-900">{fmt(detail.value)}</V>
      </td>
      <td className="py-2 text-right">
        <V className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-semibold ${scoreColor(detail.score)} ${scoreBg(detail.score)}`}>
          {detail.score.toFixed(1)}<span className="text-[10px] font-normal ml-0.5 opacity-60">/5</span>
        </V>
      </td>
    </tr>
  );
}

export function StatusPage() {
  const [info, setInfo] = useState<NodeInfo | null>(null);
  const [error, setError] = useState("");
  const [gaugeValue, setGaugeValue] = useState(0);

  const isMountedRef = useRef(true);

  useEffect(() => {
    isMountedRef.current = true;
    return () => { isMountedRef.current = false; };
  }, []);

  const fetchData = useCallback(async () => {
    try {
      const data = await api.getNodeInfo();
      if (isMountedRef.current) {
        setInfo(data);
        setError("");
      }
    } catch (err) {
      if (isMountedRef.current) {
        setError(err instanceof Error ? err.message : "Failed to load");
      }
    }
  }, []);

  // Animation
  const animatingRef = useRef(false);
  const gaugeTargetRef = useRef(0);
  const gaugeValueRef = useRef(0);

  useEffect(() => { gaugeValueRef.current = gaugeValue; }, [gaugeValue]);

  const setGauge = useCallback((v: number) => {
    gaugeValueRef.current = v;
    setGaugeValue(v);
  }, []);

  useEffect(() => {
    if (!info) return;

    const target = info.power_reserve;
    gaugeTargetRef.current = target;
    const current = gaugeValueRef.current;

    // First load: ease in from target - 10%
    if (current === 0 && target > 0 && !animatingRef.current) {
      animatingRef.current = true;
      const from = Math.min(target * 1.02, 5);
      const start = Date.now();
      const duration = 500;

      const animateIn = () => {
        const p = Math.min((Date.now() - start) / duration, 1);
        const t = -(Math.cos(Math.PI * p) - 1) / 2; // ease-in-out
        setGauge(from + (gaugeTargetRef.current - from) * t);
        if (p < 1) {
          requestAnimationFrame(animateIn);
        } else {
          animatingRef.current = false;
        }
      };
      requestAnimationFrame(animateIn);

    } else if (!animatingRef.current && Math.abs(current - target) > 0.05) {
      // Subsequent polling updates: slow dignified glide
      animatingRef.current = true;
      const from = current; // use ref value, never stale
      let start = Date.now();
      const duration = 2000;

      const animateGlide = () => {
        const p = Math.min((Date.now() - start) / duration, 1);
        const t = -(Math.cos(Math.PI * p) - 1) / 2;
        setGauge(from + (gaugeTargetRef.current - from) * t);
        if (p < 1) {
          requestAnimationFrame(animateGlide);
        } else {
          animatingRef.current = false;
        }
      };
      requestAnimationFrame(animateGlide);
    }
  }, [info, setGauge]);

  // Data fetching interval
  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  // Conditional Rendering Logic
  if (error && !info) {
    return (
      <div className="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-700">
        {error}
      </div>
    );
  }

  if (!info) {
    return <div className="text-sm text-gray-500">Loading...</div>;
  }

  const metrics = info.metrics;

  // Sort available metrics by canonical order
  const sortedMetrics = metrics
    ? METRIC_ORDER.filter((k) => k in metrics).map((k) => ({ id: k, detail: metrics[k] }))
    : [];

  return (
    <ServicePageLayout
      title="Host Status"
      description={<>Real-time system health powered by{" "}
        <a
          href="https://github.com/kookyleo/pwrzv"
          target="_blank"
          rel="noopener noreferrer"
          className="no-underline hover:underline underline-offset-2 hover:opacity-70 transition-opacity"
        >
          pwrzv
        </a>
      </>}
    >
      {/* Power Reserve gauge */}
      <div className="flex justify-center">
        <PowerReserveGauge value={gaugeValue} />
      </div>

      {/* Metrics table */}
      {sortedMetrics.length > 0 && (
        <CollapsibleCard storageKey="host_metrics" title="System Metrics">
          <div className="flex justify-end mb-2">
            <span className="text-[10px] text-gray-400">Score: 5 = idle, 0 = saturated</span>
          </div>
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-gray-200 text-left text-xs font-mono text-gray-400">
                <th className="py-1.5 pr-4 font-normal">Metric</th>
                <th className="py-1.5 pr-4 font-normal">Description</th>
                <th className="py-1.5 pr-4 font-normal text-right">Value</th>
                <th className="py-1.5 font-normal text-right">Score</th>
              </tr>
            </thead>
            <tbody>
              {sortedMetrics.map(({ id, detail }) => (
                <MetricRow key={id} id={id} detail={detail} />
              ))}
            </tbody>
          </table>
        </CollapsibleCard>
      )}
    </ServicePageLayout>
  );
}
