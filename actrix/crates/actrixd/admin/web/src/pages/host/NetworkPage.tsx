import { useEffect, useState } from "react";
import {
  RefreshCw,
  Loader2,
  Server,
  Shield,
  ArrowLeftRight,
  Radio,
  Fingerprint,
  Key,
  Check,
} from "lucide-react";
import { api, type ServiceStatus } from "../../lib/api";
import { ServicePageLayout } from "../../components/layout/ServicePageLayout";
import { CollapsibleCard } from "../../components/ui/CollapsibleCard";
import { usePlatformData } from "../platform/shared";

const formatHostPort = (host: string, port: number | string): string => {
  if (!host) return "";
  const needsBrackets = host.includes(":") && !host.startsWith("[");
  const h = needsBrackets ? `[${host}]` : host;
  return `${h}:${port}`;
};

/** Copy text to clipboard — works on HTTP (non-secure) contexts too */
function copyText(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    return navigator.clipboard.writeText(text).catch(() => fallbackCopy(text));
  }
  return fallbackCopy(text);
}

function fallbackCopy(text: string): Promise<void> {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  document.execCommand("copy");
  document.body.removeChild(ta);
  return Promise.resolve();
}

/** Inline click-to-copy code snippet — blends into prose */
function C({ c }: { c: string }) {
  const [copied, setCopied] = useState(false);
  const copy = (e: React.MouseEvent) => {
    e.stopPropagation();
    copyText(c).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  return (
    <code onClick={copy} className="inline cursor-pointer rounded bg-gray-100 px-1 py-px text-[11px] font-mono text-gray-700 hover:bg-gray-200 transition-colors" title="Click to copy">
      {c}{copied && <Check className="inline ml-0.5 h-2.5 w-2.5 text-green-600" />}
    </code>
  );
}

type TestStatus = "idle" | "testing" | "success" | "failure";

interface ServiceTestResult {
  status: TestStatus;
  message?: string;
  latency?: number;
}

const SERVICE_ICONS: Record<string, any> = {
  "stun": Shield,
  "turn": ArrowLeftRight,
  "signaling": Radio,
  "ais": Fingerprint,
  "signer": Key,
};

export function NetworkPage() {
  const [services, setServices] = useState<ServiceStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<Record<string, ServiceTestResult>>({});
  const { data: platformData } = usePlatformData();

  const fetchServices = async () => {
    try {
      setLoading(true);
      const data = await api.getServices();
      setServices(data.services);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load services");
    } finally {
      setLoading(false);
    }
  };

  const [autoTested, setAutoTested] = useState(false);

  useEffect(() => {
    fetchServices();
  }, []);

  // Auto-run all tests once services are loaded
  useEffect(() => {
    if (!autoTested && services.length > 0 && !loading) {
      setAutoTested(true);
      services.forEach(s => {
        if (s.type === 1 || s.type === 2) {
          testStunTurnService(s);
        } else {
          testHttpService(s);
        }
      });
    }
  }, [services, loading]);

  const updateTestResult = (name: string, result: ServiceTestResult) => {
    setTestResults(prev => ({ ...prev, [name]: result }));
  };
  

  const testHttpService = async (service: ServiceStatus) => {
    const name = service.name;
    updateTestResult(name, { status: "testing" });

    try {
      const data = await api.probePort(service.port || 80);
      if (data.reachable) {
        updateTestResult(name, { status: "success", message: "Reachable", latency: data.latency_ms });
      } else {
        updateTestResult(name, { status: "failure", message: data.error || "Unreachable" });
      }
    } catch (err) {
      updateTestResult(name, {
        status: "failure",
        message: err instanceof Error ? err.message : "Connection failed"
      });
    }
  };

  // Test UDP reachability via STUN binding request (shared by STUN & TURN on same port)
  const testedPorts = new Set<number>();
  const testStunTurnService = async (service: ServiceStatus) => {
    const port = service.port || 3478;

    // Only test each UDP port once — apply result to all services on that port
    if (testedPorts.has(port)) return;
    testedPorts.add(port);

    const samePort = services.filter(s => s.port === port && (s.type === 1 || s.type === 2));
    samePort.forEach(s => updateTestResult(s.name, { status: "testing" }));
    const startTime = performance.now();

    try {
        const iceServerUrl = `stun:${window.location.hostname}:${port}`;
        const pc = new RTCPeerConnection({ iceServers: [{ urls: iceServerUrl }] });
        pc.createDataChannel("test");
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);

        const result = await new Promise<string>((resolve, reject) => {
            const timeout = setTimeout(() => {
                pc.close();
                reject(new Error("No STUN response (timeout)"));
            }, 5000);

            pc.onicecandidate = (event) => {
                if (event.candidate) {
                    const t = event.candidate.type;
                    if (t === "srflx" || t === "relay" || t === "prflx") {
                        clearTimeout(timeout);
                        pc.close();
                        resolve(t);
                    }
                } else {
                    // gathering done, null candidate
                }
            };

            pc.onicegatheringstatechange = () => {
                if (pc.iceGatheringState === "complete") {
                    clearTimeout(timeout);
                    pc.close();
                    reject(new Error("No reflexive candidate"));
                }
            };
        });

        const duration = Math.round(performance.now() - startTime);
        samePort.forEach(s => updateTestResult(s.name, {
            status: "success", message: `${result} candidate`, latency: duration
        }));
    } catch (err) {
        samePort.forEach(s => updateTestResult(s.name, {
            status: "failure",
            message: err instanceof Error ? err.message : "ICE check failed"
        }));
    }
  };

  const runTest = (service: ServiceStatus) => {
    if (service.type === 1 || service.type === 2) {
        testStunTurnService(service);
    } else {
        testHttpService(service);
    }
  };


  const runAllTests = () => {
    testedPorts.clear();
    services.forEach(runTest);
  };

  if (loading) return <div className="flex h-full items-center justify-center"><Loader2 className="h-8 w-8 animate-spin text-blue-600" /></div>;
  if (error) return <div className="p-8 text-center text-red-600">{error}</div>;

  // All expected services — ones missing from API are treated as disabled
  const ALL_SERVICES: { name: string; type: number; port: number }[] = [
    { name: "Signer Service", type: 5, port: 80 },
    { name: "AIS Service", type: 4, port: 80 },
    { name: "Signaling Service", type: 3, port: 80 },
    { name: "STUN Server", type: 1, port: 3478 },
    { name: "TURN Server", type: 2, port: 3478 },
  ];

  const apiMap = new Map(services.map(s => [s.name, s]));

  type ExtendedService = ServiceStatus & { enabled: boolean };
  const allServices: ExtendedService[] = ALL_SERVICES.map(def => {
    const s = apiMap.get(def.name);
    if (s) return { ...s, enabled: true };
    return {
      name: def.name, type: def.type, port: def.port,
      is_healthy: false, active_connections: 0, total_requests: 0,
      failed_requests: 0, average_latency_ms: 0, url: null, domain: null,
      enabled: false,
    };
  });

  // Group services by port
  const portGroups: Record<number, ExtendedService[]> = {};
  allServices.forEach(s => {
      if (s.port) {
          if (!portGroups[s.port]) portGroups[s.port] = [];
          portGroups[s.port].push(s);
      }
  });
  

  // ── Diagram layout ──────────────────────────────────────────
  const groupKeys = Object.keys(portGroups).map(Number).sort((a, b) => a - b);
  const rowCount = groupKeys.length;

  // Pill zone (left)
  const PILL_W = 105, PILL_H = 42, PILL_GAP = 10;
  // Port-bar zone (center)
  const BAR_H = 26;
  const FW_X = 510;                            // center line of firewall/boundary
  const BOUNDARY_OFFSET = 10;                  // horizontal offset for entire boundary
  const effectiveFW_X = FW_X + BOUNDARY_OFFSET; // actual center after offset
  const BAR_INT_LEFT = 390;                    // internal bar left edge
  const BAR_EXT_RIGHT = 770;                   // external bar right edge
  // Rows
  const ROW_H = 80;
  const FIRST_ROW_CY = 60;
  const SVG_W = 900;
  const FW_WALL_TOP = 12;
  const SVG_H = FIRST_ROW_CY + Math.max(0, rowCount - 1) * ROW_H + 64;
  const rowCY = (i: number) => FIRST_ROW_CY + i * ROW_H;
  const LABEL_X = effectiveFW_X + 50;      // right-side text just outside the frame

  // Short pill labels — strip verbose suffixes
  const shortLabel = (name: string): string => {
    return name.replace(/\s+(Service|Server)$/i, "");
  };

  // Colors
  const CG = "#2d6a2e";                        // dark green (success)
  const CR = "#a63028";                        // muted red  (failure)
  const CB = "#2563eb";                        // blue       (testing)
  const CN = "#94a3b8";                        // neutral gray (idle/untested)

  const NAME_ORDER: Record<string, number> = { "Signer": 0, "Signer Service": 0, "AIS": 1, "AIS Service": 1, "Signaling": 2, "Signaling Service": 2, "STUN": 3, "STUN Server": 3, "TURN": 4, "TURN Server": 4 };
  const sortedServices = [...allServices].sort((a, b) => (NAME_ORDER[a.name] ?? 99) - (NAME_ORDER[b.name] ?? 99));

  // Pill: service health — disabled → gray, healthy → green, unhealthy → red
  const pillFill = (s: ExtendedService) => {
    if (!s.enabled) return CN;
    return s.is_healthy ? CG : CR;
  };

  // Internal bar: port is listening → green; all disabled → gray
  const intBarFill = (svcs: ServiceStatus[]) => {
    if (svcs.some(s => s.is_healthy)) return CG;
    return CN;
  };

  // External bar: browser test result
  const extBarFill = (svcs: ServiceStatus[]) => {
    if (svcs.some(s => testResults[s.name]?.status === "failure")) return CR;
    if (svcs.some(s => testResults[s.name]?.status === "testing")) return CB;
    if (svcs.every(s => testResults[s.name]?.status === "success")) return CG;
    return CN;
  };

  // Labels beside ports
  const domainHost = typeof window !== "undefined" ? window.location.hostname : "";
  const advIp = platformData?.config_fields.find(f => f.key === "bind.ice.advertised_ip")?.effective_value ?? "";
  const advPort = platformData?.config_fields.find(f => f.key === "bind.ice.advertised_port")?.effective_value ?? "";
  const relayPortMin = platformData?.config_fields.find(f => f.key === "bind.ice.relay_port_min")?.effective_value ?? "49152";
  const relayPortMax = platformData?.config_fields.find(f => f.key === "bind.ice.relay_port_max")?.effective_value ?? "65535";

  const isTesting = (svcs: ExtendedService[]) =>
    svcs.some(s => testResults[s.name]?.status === "testing");
  const svcTesting = (s: ExtendedService) =>
    testResults[s.name]?.status === "testing";

  return (
    <ServicePageLayout
      title="Network Diagnostics"
      description="Visual network topology and firewall status."
      headerActions={
        <button onClick={runAllTests} className="inline-flex items-center rounded-md bg-white px-4 py-2 text-sm font-medium text-gray-700 shadow-sm border border-gray-300 hover:bg-gray-50 active:scale-95 active:bg-gray-100 transition-transform duration-75">
          <RefreshCw className="mr-2 h-4 w-4" /> retest
        </button>
      }
    >

      <div className="flex flex-col items-center overflow-x-auto">
        <svg viewBox={`0 -40 ${SVG_W} ${SVG_H + 40}`} className="max-w-3xl mx-auto select-none">
          <defs>
            <style>{`
              @keyframes pulse { 0%,100% { opacity: 1 } 50% { opacity: 0.35 } }
              .testing { animation: pulse 1.2s ease-in-out infinite; }
            `}</style>
          </defs>
          {/* ── Network boundary zone ── */}
          {(() => {
            const zoneW = 70;
            const zoneH = SVG_H - 36;
            const zoneX = effectiveFW_X - zoneW / 2;
            const outerY = FW_WALL_TOP;
            const backExpand = 10;
            const cellH = zoneH / 3;
            const foW = 44; // foreignObject width for composition
            const foH = 34; // foreignObject height
            const foX = effectiveFW_X - foW / 2;
            const cellCY = (i: number) => FW_WALL_TOP + cellH * i + cellH / 2;

            return (
              <>
                {/* Subtle layered back plate for depth */}
                <rect
                  x={zoneX - backExpand / 2}
                  y={outerY - backExpand / 2}
                  width={zoneW + backExpand}
                  height={zoneH + backExpand}
                  rx={7}
                  fill="#eff3fb"
                  stroke="#d5ddf0"
                  strokeWidth="1"
                  opacity="0.9"
                />
                {/* Single flat frame with subtle fill */}
                <rect
                  x={zoneX}
                  y={outerY}
                  width={zoneW}
                  height={zoneH}
                  rx={6}
                  fill="#f8fafc"
                  stroke="#cbd5e1"
                  strokeWidth="1.1"
                />
                {/* Label just above the frame */}
                <text
                  x={effectiveFW_X}
                  y={FW_WALL_TOP - 11}
                  textAnchor="middle"
                  fontSize="10"
                  fontWeight="600"
                  fill="#94a3b8"
                  letterSpacing="0.08em"
                >
                  NETWORK BOUNDARY
                </text>
                {/* NAT */}
                <foreignObject x={foX} y={cellCY(0) - foH / 2} width={foW} height={foH}>
                  <div className="flex items-center justify-center h-full">
                    <ArrowLeftRight className="h-6 w-6 text-slate-400" />
                  </div>
                </foreignObject>
                {/* Firewall */}
                <foreignObject x={foX} y={cellCY(1) - foH / 2} width={foW} height={foH}>
                  <div className="flex items-center justify-center h-full">
                    <Shield className="h-6 w-6 text-slate-400" />
                  </div>
                </foreignObject>
                {/* Firewall */}
                <foreignObject x={foX} y={cellCY(2) - foH / 2} width={foW} height={foH}>
                  <div className="flex items-center justify-center h-full">
                    <Shield className="h-6 w-6 text-slate-400" />
                  </div>
                </foreignObject>
              </>
            );
          })()}


          {/* ── Port-group rows ── */}
          {groupKeys.map((port, idx) => {
            const svcs = [...portGroups[port]].sort((a, b) => (NAME_ORDER[a.name] ?? 99) - (NAME_ORDER[b.name] ?? 99));
            const cy = rowCY(idx);
            const iFill = intBarFill(svcs);
            const eFill = extBarFill(svcs);
            const testing = isTesting(svcs);
            const extClass = testing ? "testing" : undefined;

            return (
              <g key={port}>
                {/* Service pills */}
                {svcs.map((s, si) => {
                  const pillRight = BAR_INT_LEFT - 16;
                  const px = pillRight - (svcs.length - si) * (PILL_W + PILL_GAP) + PILL_GAP;
                  const py = cy - PILL_H / 2;
                  const color = pillFill(s);
                  const Icon = SERVICE_ICONS[s.name.toLowerCase()] || Server;

                  const showReason = true;

                  return (
                    <g key={s.name}>
                      <rect x={px} y={py} width={PILL_W} height={PILL_H} rx="14" fill="none" stroke={color} strokeWidth="1" className={svcTesting(s) ? "testing" : undefined} />
                      <foreignObject x={px} y={py} width={PILL_W} height={PILL_H}>
                        <div className="flex flex-col items-center justify-center h-full px-2" style={{ gap: showReason ? "1px" : "0" }}>
                          <div className="flex items-center gap-1.5">
                            <Icon className="h-3.5 w-3.5 shrink-0" strokeWidth={2} style={{ color }} />
                            <span className="text-[11px] font-semibold truncate" style={{ color }}>{shortLabel(s.name)}</span>
                          </div>
                          {showReason && (
                            <span className="text-[8px] truncate max-w-full opacity-80" style={{ color }}>{!s.enabled ? "Disabled" : s.is_healthy ? "Healthy" : "Unhealthy"}</span>
                          )}
                        </div>
                      </foreignObject>
                    </g>
                  );
                })}

                {/* Bars and port labels */}
                {port === 3478 ? (
                  <>
                    {/* Thin 2px line for :3478 */}
                    <rect x={BAR_INT_LEFT} y={cy - 10} width={effectiveFW_X - BAR_INT_LEFT - 35} height={2} fill={iFill} />
                    <rect x={effectiveFW_X + 35} y={cy - 10} width={BAR_EXT_RIGHT - effectiveFW_X - 35} height={2} fill={eFill} className={extClass} />
                    <text x={effectiveFW_X - 50} y={cy - 14} textAnchor="end" fontSize="10" fontWeight="600" fontFamily="monospace" fill="#64748b">3478/udp</text>
                    {advIp && advPort && (
                      <text x={LABEL_X} y={cy - 14} textAnchor="start" fontSize="10" fontWeight="600" fill="#334155">
                        {formatHostPort(advIp, advPort)}
                      </text>
                    )}
                    {advIp && (
                      <text x={LABEL_X} y={cy + 21} textAnchor="start" fontSize="10" fontWeight="600" fill="#334155">
                        {`:${relayPortMin}–${relayPortMax}`}
                      </text>
                    )}

                    {/* Wide 16px bar for relay port range */}
                    <rect x={BAR_INT_LEFT} y={cy - 5} width={effectiveFW_X - BAR_INT_LEFT - 35} height={16} fill={iFill} />
                    <rect x={effectiveFW_X + 35} y={cy - 5} width={BAR_EXT_RIGHT - effectiveFW_X - 35} height={16} fill={eFill} className={extClass} />
                    <text x={effectiveFW_X - 50} y={cy + 21} textAnchor="end" fontSize="10" fontWeight="600" fontFamily="monospace" fill="#64748b">{`${relayPortMin}–${relayPortMax}/udp`}</text>
                  </>
                ) : (
                  <>
                    {(() => { const h = port === 80 ? 2 : BAR_H; return (
                      <>
                        <rect x={BAR_INT_LEFT} y={cy - h / 2} width={effectiveFW_X - BAR_INT_LEFT - 35} height={h} fill={iFill} />

                        <rect x={effectiveFW_X + 35} y={cy - h / 2} width={BAR_EXT_RIGHT - effectiveFW_X - 35} height={h} fill={eFill} className={extClass} />
                      </>
                    ); })()}
                    <text x={effectiveFW_X - 50} y={port === 80 ? cy - 5 : cy + 4} textAnchor="end" fontSize="10" fontWeight="600" fontFamily="monospace" fill={port === 80 ? "#64748b" : "white"}>
                      {`${port}/${port === 80 ? "tcp" : "udp"}`}
                    </text>
                    {port === 80 && domainHost && (
                      <text x={LABEL_X} y={cy - 5} textAnchor="start" fontSize="10" fontWeight="600" fill="#334155">
                        {formatHostPort(domainHost, port)}
                      </text>
                    )}
                  </>
                )}
              </g>
            );
          })}
        </svg>

      </div>

      {/* ── Service Health ── */}
      {(() => {
        const hasIssue = sortedServices.some(s => s.enabled && !s.is_healthy);
        return (
        <CollapsibleCard storageKey="net_health" title="Service Health">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-gray-200 text-left text-xs font-mono text-gray-400">
                <th className="py-1.5 pr-4 font-normal w-28">Service</th>
                <th className="py-1.5 pr-4 font-normal w-24">Port</th>
                <th className="py-1.5 pr-4 font-normal w-20">Status</th>
                {hasIssue && <th className="py-1.5 font-normal">Troubleshoot</th>}
              </tr>
            </thead>
            <tbody>
              {sortedServices.map(s => {
                const proto = s.type === 1 || s.type === 2 ? "UDP" : "TCP";
                const label = shortLabel(s.name);
                return (
                  <tr key={s.name} className="border-b border-gray-100 last:border-0 align-top">
                    <td className="py-2 pr-4 font-medium text-gray-900">{label}</td>
                    <td className="py-2 pr-4 font-mono text-gray-600">
                      {s.port}/{proto.toLowerCase()}
                      {s.type === 2 && <>, 49152–65535/udp</>}
                    </td>
                    <td className="py-2 pr-4">
                      {!s.enabled ? (
                        <span className="text-gray-400 text-xs font-medium">Disabled</span>
                      ) : s.is_healthy ? (
                        <span className="text-green-700 text-xs font-medium">Healthy</span>
                      ) : (
                        <span className="text-red-700 text-xs font-medium">Unhealthy</span>
                      )}
                    </td>
                    {hasIssue && (
                      <td className="py-2 text-xs text-gray-500 leading-relaxed">
                        {s.enabled && !s.is_healthy && (<>
                          {label} failed to initialize. Check logs
                          with <C c="journalctl -u actrix -n 50 --no-pager" /> and
                          verify port binding via <C c={`sudo ss -tlnp sport = :${s.port}`} />.
                          Review <C c="cat /etc/actrix/config.toml" /> for config errors.
                        </>)}
                      </td>
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </CollapsibleCard>
        );
      })()}

      {/* ── Firewall / External Reachability ── */}
      {(() => {
        const fwGroups = Object.keys(portGroups).map(Number).sort((a, b) => a - b);
        const hasFirewallIssue = fwGroups.some(port =>
          portGroups[port].some(s => testResults[s.name]?.status === "failure")
        );
        return (
        <CollapsibleCard storageKey="net_firewall" title="Firewall">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-gray-200 text-left text-xs font-mono text-gray-400">
                <th className="py-1.5 pr-4 font-normal w-28">Port</th>
                <th className="py-1.5 pr-4 font-normal w-28">Services</th>
                <th className="py-1.5 pr-4 font-normal w-20">Status</th>
                {hasFirewallIssue && <th className="py-1.5 font-normal">Troubleshoot</th>}
              </tr>
            </thead>
            <tbody>
              {fwGroups.map(port => {
                const svcs = portGroups[port];
                const proto = svcs[0].type === 1 || svcs[0].type === 2 ? "UDP" : "TCP";
                const protoLower = proto.toLowerCase();
                const names = svcs.map(s => shortLabel(s.name)).join(", ");
                const isTesting = svcs.some(s => testResults[s.name]?.status === "testing");
                const isFail = svcs.some(s => testResults[s.name]?.status === "failure");
                const isSuccess = svcs.every(s => testResults[s.name]?.status === "success");
                const isIce = svcs[0].type === 1 || svcs[0].type === 2;
                const hostname = window.location.hostname;
                return (
                  <tr key={port} className="border-b border-gray-100 last:border-0 align-top">
                    <td className="py-2 pr-4 font-mono text-gray-600">{port}/{protoLower}</td>
                    <td className="py-2 pr-4 text-gray-900">{names}</td>
                    <td className="py-2 pr-4">
                      {isTesting ? (
                        <span className="flex items-center gap-1 text-blue-600 text-xs"><Loader2 className="h-3 w-3 animate-spin" />Testing</span>
                      ) : isSuccess ? (
                        <span className="text-green-700 text-xs font-medium">Reachable</span>
                      ) : isFail ? (
                        <span className="text-red-700 text-xs font-medium">Unreachable</span>
                      ) : (
                        <span className="text-gray-400 text-xs">—</span>
                      )}
                    </td>
                    {hasFirewallIssue && (
                      <td className="py-2 text-xs text-gray-500 leading-relaxed">
                        {isFail && (<>
                          Inbound {protoLower} blocked. Open
                          with <C c={`sudo ufw allow ${port}/${protoLower}`} /> and
                          verify via <C c={isIce ? `stun ${hostname} ${port}` : `curl -sI http://${hostname}:${port}`} />.
                          Check rules: <C c={`sudo ufw status | grep ${port}`} />.
                          {isIce && port === 3478 && <>{" "}Relay range
                            also needed: <C c="sudo ufw allow 49152:65535/udp" />.</>}
                          {isIce && <>{" "}Also verify
                            that <C c="bind.ice.advertised_ip" /> in <C c="/etc/actrix/config.toml" /> matches
                            the host's actual public IP: <C c="curl -s ifconfig.me" />.</>}
                        </>)}
                      </td>
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </CollapsibleCard>
        );
      })()}
    </ServicePageLayout>
  );
}
