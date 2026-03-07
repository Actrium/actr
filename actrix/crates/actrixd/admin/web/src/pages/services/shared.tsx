import { Fragment, useEffect, useState, useMemo, useRef, useCallback } from "react";
import { Link } from "react-router-dom";
import { Pencil } from "lucide-react";
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer } from "recharts";
import { api, type ResolvedField, type ServiceStatus, type MetricSample } from "../../lib/api";
import { ConfigFieldModal } from "../../components/ui/ConfigFieldModal";
import { CollapsibleCard } from "../../components/ui/CollapsibleCard";

// ── StatusBadge ────────────────────────────────────────────────

export function StatusBadge({
  enabled,
  healthy,
}: {
  enabled: boolean;
  healthy?: boolean;
}) {
  if (!enabled) {
    return (
      <span className="inline-flex items-center rounded-full bg-gray-100 px-3 py-1 text-xs font-medium text-gray-600">
        Disabled
      </span>
    );
  }
  if (healthy) {
    return (
      <span className="inline-flex items-center rounded-full bg-green-100 px-3 py-1 text-xs font-medium text-green-700">
        Healthy
      </span>
    );
  }
  return (
    <span className="inline-flex items-center rounded-full bg-red-100 px-3 py-1 text-xs font-medium text-red-700">
      Unhealthy
    </span>
  );
}

// ── Source indicator dot ────────────────────────────────────────

function SourceDot({ source }: { source: string }) {
  const colors: Record<string, string> = {
    default: "bg-gray-300",
    config_file: "bg-blue-500",
    override: "bg-amber-500",
  };
  const labels: Record<string, string> = {
    default: "Default value",
    config_file: "Set in config file",
    override: "Dynamic override",
  };
  return (
    <span
      className={`inline-block w-2 h-2 rounded-full ${colors[source] ?? "bg-gray-300"}`}
      title={labels[source] ?? source}
    />
  );
}

// ── ConfigTable with [edit] links ──────────────────────────────

export interface TomlMapping {
  section: string;
  keys: Record<string, string>;
}

function findLineNumbers(
  content: string,
  mappings: TomlMapping[],
): Record<string, number> {
  const lines = content.split("\n");
  const result: Record<string, number> = {};

  for (const { section, keys } of mappings) {
    let sectionStart = -1;
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].trim() === section) {
        sectionStart = i;
        break;
      }
    }
    if (sectionStart === -1) continue;

    for (let i = sectionStart + 1; i < lines.length; i++) {
      const trimmed = lines[i].trim();
      if (trimmed.startsWith("[") && trimmed !== section) break;

      for (const [jsonKey, tomlKey] of Object.entries(keys)) {
        if (result[jsonKey]) continue;
        const re = new RegExp(`^${escapeRegex(tomlKey)}\\s*=`);
        if (re.test(trimmed)) {
          result[jsonKey] = i + 1;
        }
      }
    }
  }

  return result;
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// ── New: ResolvedField-based ConfigTable ────────────────────────

export function ConfigFieldsTable({
  fields,
  onRefresh,
}: {
  fields: ResolvedField[];
  onRefresh?: () => void;
}) {
  const [editField, setEditField] = useState<ResolvedField | null>(null);
  const [lineMap, setLineMap] = useState<Record<string, number>>({});

  // Fetch config file content to find TOML line numbers
  useEffect(() => {
    api
      .getConfigFile()
      .then(({ content }) => {
        setLineMap(findLineNumbersForFields(content, fields));
      })
      .catch(() => {});
  }, [fields]);

  // Group fields by toml_path prefix (section)
  const grouped = groupFieldsBySection(fields);

  return (
    <>
      <div className="overflow-x-auto">
        <table className="w-full text-sm table-fixed">
          <colgroup>
            <col className="w-[28%]" />
            <col className="w-auto" />
            <col className="w-[30%]" />
            <col className="w-8" />
          </colgroup>
          <tbody>
            {grouped.map(({ section, fields: sectionFields }) => (
              <Fragment key={section}>
                <tr>
                  <td
                    colSpan={4}
                    className="py-2 text-xs font-mono text-gray-400 bg-gray-50"
                  >
                    [{section}]
                  </td>
                </tr>
                {sectionFields.map((field) => (
                  <tr
                    key={field.key}
                    className="border-b border-gray-100 last:border-0"
                  >
                    <td className="py-2 pr-3 font-medium text-gray-500 truncate" title={field.key}>
                      {fieldDisplayKey(field.key)}
                    </td>
                    <td className="py-2 pr-3 text-xs text-gray-400 truncate" title={field.description}>
                      {field.description}
                    </td>
                    <td className="py-2 font-mono text-gray-900 truncate" title={field.effective_value}>
                      <span className="inline-flex items-center gap-1.5">
                        <SourceDot source={field.source} />
                        <span className="truncate">{field.effective_value}</span>
                      </span>
                    </td>
                    <td className="py-2 pl-2 text-right">
                      <button
                        onClick={() => setEditField(field)}
                        className="inline-flex items-center text-gray-400 hover:text-blue-600 transition-colors"
                        title="Edit config override"
                      >
                        <Pencil className="h-3 w-3" />
                      </button>
                    </td>
                  </tr>
                ))}
              </Fragment>
            ))}
          </tbody>
        </table>
      </div>

      {editField && (
        <ConfigFieldModal
          field={editField}
          tomlLine={lineMap[editField.key]}
          onClose={() => setEditField(null)}
          onSaved={() => {
            setEditField(null);
            onRefresh?.();
          }}
        />
      )}
    </>
  );
}

/**
 * Find TOML line numbers for resolved fields by matching their key's
 * last segment against lines in the appropriate TOML section.
 */
function findLineNumbersForFields(
  content: string,
  fields: ResolvedField[],
): Record<string, number> {
  const lines = content.split("\n");
  const result: Record<string, number> = {};

  for (const field of fields) {
    // Only bother for fields that have a config_file_value
    if (field.config_file_value == null) continue;

    const lastDot = field.key.lastIndexOf(".");

    if (lastDot < 0) {
      // Top-level key (no section): search from start until first [section]
      const re = new RegExp(`^${escapeRegex(field.key)}\\s*=`);
      for (let i = 0; i < lines.length; i++) {
        const trimmed = lines[i].trim();
        if (trimmed.startsWith("[")) break;
        if (re.test(trimmed)) {
          result[field.key] = i + 1;
          break;
        }
      }
      continue;
    }

    // field.key like "services.signaling.server.rate_limit.connection.per_minute"
    // TOML section: [services.signaling.server.rate_limit.connection]
    // TOML key: per_minute
    const sectionPath = field.key.substring(0, lastDot);
    const leafKey = field.key.substring(lastDot + 1);

    // Find the section header — could be [section] or [section.sub]
    const sectionHeader = `[${sectionPath}]`;
    let sectionStart = -1;
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].trim() === sectionHeader) {
        sectionStart = i;
        break;
      }
    }
    if (sectionStart === -1) continue;

    // Search within the section for the key
    const re = new RegExp(`^${escapeRegex(leafKey)}\\s*=`);
    for (let i = sectionStart + 1; i < lines.length; i++) {
      const trimmed = lines[i].trim();
      if (trimmed.startsWith("[") && trimmed !== sectionHeader) break;
      if (re.test(trimmed)) {
        result[field.key] = i + 1; // 1-based line number
        break;
      }
    }
  }

  return result;
}

/** Group fields by TOML section prefix */
function groupFieldsBySection(
  fields: ResolvedField[],
): { section: string; fields: ResolvedField[] }[] {
  const groups: Map<string, ResolvedField[]> = new Map();

  for (const field of fields) {
    // e.g. "services.signaling.server.rate_limit.connection.enabled" -> section = "services.signaling.server.rate_limit.connection"
    const lastDot = field.key.lastIndexOf(".");
    const section = lastDot > 0 ? field.key.substring(0, lastDot) : field.key;

    if (!groups.has(section)) {
      groups.set(section, []);
    }
    groups.get(section)!.push(field);
  }

  return Array.from(groups.entries()).map(([section, fields]) => ({
    section,
    fields,
  }));
}

/** Extract the last segment of a dotted key for display */
function fieldDisplayKey(key: string): string {
  const lastDot = key.lastIndexOf(".");
  return lastDot > 0 ? key.substring(lastDot + 1) : key;
}

// ── ServiceMetrics ──────────────────────────────────────────────

const TIME_RANGES = [
  { key: "15m", label: "15min", tier: 0, timeFmt: (_s: MetricSample, i: number) => `${i}m` },
  { key: "4h",  label: "4h",    tier: 1, timeFmt: (_s: MetricSample, i: number) => `${i * 15}m` },
  { key: "72h", label: "72h",   tier: 2, timeFmt: (_s: MetricSample, i: number) => `${i * 4}h` },
] as const;

function samplesToChartData(samples: MetricSample[], timeFmt: (_s: MetricSample, i: number) => string) {
  return samples.map((s, i) => {
    const successPct = s.requests > 0 ? ((s.requests - s.failed_requests) / s.requests) * 100 : 100;
    return {
      time: timeFmt(s, i),
      connections: s.active_conns,
      requests: s.requests,
      p95_ms: Math.round(s.latency_p95_ms * 10) / 10,
      success_pct: Math.round(successPct * 10) / 10,
    };
  });
}

export function ServiceMetrics({
  status,
  storageKey,
}: {
  status: ServiceStatus | null;
  storageKey: string;
}) {
  const type = status?.type ?? 0;
  const [rangeIdx, setRangeIdx] = useState(0);
  const [samples, setSamples] = useState<Record<number, MetricSample[]>>({});
  const initialRef = useRef(true);
  const range = TIME_RANGES[rangeIdx];

  const fetchTimeseries = useCallback(async () => {
    // Fetch all tiers in parallel
    const results = await Promise.all(
      TIME_RANGES.map((r) => api.getMetricsTimeseries(type, r.tier).catch(() => null)),
    );
    const map: Record<number, MetricSample[]> = {};
    results.forEach((res, i) => {
      if (res?.samples) map[TIME_RANGES[i].tier] = res.samples;
    });
    setSamples(map);
  }, [type]);

  // Initial fetch + 30s refresh
  useEffect(() => {
    fetchTimeseries();
    const id = setInterval(() => {
      initialRef.current = false;
      fetchTimeseries();
    }, 30_000);
    return () => clearInterval(id);
  }, [fetchTimeseries]);

  const tierSamples = samples[range.tier] ?? [];
  const chartData = useMemo(
    () => samplesToChartData(tierSamples, range.timeFmt),
    [tierSamples, range],
  );

  const animate = !initialRef.current;

  const cb = { dash: "3 3", grid: "#f1f5f9", axis: "#cbd5e1" };
  const tip = { fontSize: 11, borderRadius: 8, border: "1px solid #e2e8f0" };

  return (
    <CollapsibleCard title="Metrics" storageKey={`metrics_${storageKey}`}>
      <div className="mb-4">
        <div className="inline-flex rounded-lg bg-gray-100 p-0.5 text-[11px]">
          {TIME_RANGES.map((r, i) => (
            <button
              key={r.key}
              onClick={() => setRangeIdx(i)}
              className={`px-2.5 py-0.5 rounded-md font-medium transition-colors ${
                i === rangeIdx ? "bg-white text-gray-900 shadow-sm" : "text-gray-500 hover:text-gray-700"
              }`}
            >
              {r.label}
            </button>
          ))}
        </div>
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-4">
        {/* Connections */}
        <div className="rounded-xl bg-gray-50 p-4">
          <p className="text-[10px] text-gray-400 mb-1">Connections</p>
          <ResponsiveContainer width="100%" height={140}>
            <LineChart data={chartData}>
              <CartesianGrid strokeDasharray={cb.dash} stroke={cb.grid} />
              <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke={cb.axis} interval="preserveStartEnd" />
              <YAxis tick={{ fontSize: 9 }} stroke={cb.axis} width={35} />
              <Tooltip contentStyle={tip} />
              <Line type="monotone" dataKey="connections" stroke="#3b82f6" strokeWidth={2} dot={false} isAnimationActive={animate} animationDuration={600} />
            </LineChart>
          </ResponsiveContainer>
        </div>

        {/* Success Rate */}
        <div className="rounded-xl bg-gray-50 p-4">
          <p className="text-[10px] text-gray-400 mb-1">Success Rate</p>
          <ResponsiveContainer width="100%" height={140}>
            <LineChart data={chartData}>
              <CartesianGrid strokeDasharray={cb.dash} stroke={cb.grid} />
              <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke={cb.axis} interval="preserveStartEnd" />
              <YAxis tick={{ fontSize: 9 }} stroke={cb.axis} width={35} unit="%" domain={[95, 100]} />
              <Tooltip contentStyle={tip} />
              <Line type="monotone" dataKey="success_pct" name="success %" stroke="#22c55e" strokeWidth={2} dot={false} isAnimationActive={animate} animationDuration={600} />
            </LineChart>
          </ResponsiveContainer>
        </div>

        {/* p95 Latency */}
        <div className="rounded-xl bg-gray-50 p-4">
          <p className="text-[10px] text-gray-400 mb-1">p95 Latency</p>
          <ResponsiveContainer width="100%" height={140}>
            <LineChart data={chartData}>
              <CartesianGrid strokeDasharray={cb.dash} stroke={cb.grid} />
              <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke={cb.axis} interval="preserveStartEnd" />
              <YAxis tick={{ fontSize: 9 }} stroke={cb.axis} width={35} unit="ms" />
              <Tooltip contentStyle={tip} />
              <Line type="monotone" dataKey="p95_ms" name="p95" stroke="#f59e0b" strokeWidth={2} dot={false} isAnimationActive={animate} animationDuration={600} />
            </LineChart>
          </ResponsiveContainer>
        </div>

        {/* Requests */}
        <div className="rounded-xl bg-gray-50 p-4">
          <p className="text-[10px] text-gray-400 mb-1">Requests</p>
          <ResponsiveContainer width="100%" height={140}>
            <LineChart data={chartData}>
              <CartesianGrid strokeDasharray={cb.dash} stroke={cb.grid} />
              <XAxis dataKey="time" tick={{ fontSize: 9 }} stroke={cb.axis} interval="preserveStartEnd" />
              <YAxis tick={{ fontSize: 9 }} stroke={cb.axis} width={35} />
              <Tooltip contentStyle={tip} />
              <Line type="monotone" dataKey="requests" stroke="#8b5cf6" strokeWidth={2} dot={false} isAnimationActive={animate} animationDuration={600} />
            </LineChart>
          </ResponsiveContainer>
        </div>
      </div>
    </CollapsibleCard>
  );
}

// ── Legacy ConfigTable (backward compatible) ───────────────────

export function ConfigTable({
  config,
  tomlMappings,
  descriptions,
}: {
  config: Record<string, unknown>;
  tomlMappings?: TomlMapping[];
  descriptions?: Record<string, string>;
}) {
  const [lineMap, setLineMap] = useState<Record<string, number>>({});

  useEffect(() => {
    if (!tomlMappings || tomlMappings.length === 0) return;
    api
      .getConfigFile()
      .then(({ content }) => {
        setLineMap(findLineNumbers(content, tomlMappings));
      })
      .catch(() => {});
  }, [tomlMappings]);

  const hasDescriptions = descriptions && Object.keys(descriptions).length > 0;
  const colCount = hasDescriptions ? 4 : 3;

  const renderRow = (key: string, value: unknown) => {
    const line = lineMap[key];
    const displayValue = typeof value === "object" ? JSON.stringify(value) : String(value);
    return (
      <tr key={key} className="border-b border-gray-100 last:border-0">
        <td className="py-2 pr-3 font-medium text-gray-500 truncate" title={key}>
          {key}
        </td>
        {hasDescriptions && (
          <td className="py-2 pr-3 text-xs text-gray-400 truncate" title={descriptions[key] ?? ""}>
            {descriptions[key] ?? ""}
          </td>
        )}
        <td className="py-2 font-mono text-gray-900 truncate" title={displayValue}>
          {displayValue}
        </td>
        <td className="py-2 pl-2 text-right">
          {line ? (
            <Link
              to={`/admin/config?edit#l1:${key}`}
              className="inline-flex items-center text-gray-400 hover:text-blue-600 transition-colors"
              title={`Edit in config.toml`}
            >
              <Pencil className="h-3 w-3" />
            </Link>
          ) : null}
        </td>
      </tr>
    );
  };

  const colGroup = hasDescriptions ? (
    <colgroup>
      <col className="w-[28%]" />
      <col className="w-auto" />
      <col className="w-[30%]" />
      <col className="w-8" />
    </colgroup>
  ) : (
    <colgroup>
      <col className="w-[35%]" />
      <col className="w-auto" />
      <col className="w-8" />
    </colgroup>
  );

  if (tomlMappings && tomlMappings.length > 0) {
    return (
      <div className="overflow-x-auto">
        <table className="w-full text-sm table-fixed">
          {colGroup}
          <tbody>
            {tomlMappings.map(({ section, keys }) => {
              const sectionKeys = Object.keys(keys).filter(
                (k) => k in config,
              );
              if (sectionKeys.length === 0) return null;
              return (
                <Fragment key={section}>
                  <tr>
                    <td
                      colSpan={colCount}
                      className="py-2 text-xs font-mono text-gray-400 bg-gray-50"
                    >
                      {section}
                    </td>
                  </tr>
                  {sectionKeys.map((key) => renderRow(key, config[key]))}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
    );
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm table-fixed">
        {colGroup}
        <tbody>
          {Object.entries(config).map(([key, value]) => renderRow(key, value))}
        </tbody>
      </table>
    </div>
  );
}
