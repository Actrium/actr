import { useEffect, useState, useCallback } from "react";
import { api, type ServiceDetail, type KeyEntry } from "../../lib/api";
import { ServicePageLayout, ConfigSection, StatusSection } from "../../components/layout/ServicePageLayout";
import { HowItWorks } from "../../components/ui/HowItWorks";
import { ServiceMetrics } from "./shared";
import { CollapsibleCard } from "../../components/ui/CollapsibleCard";

function AisDiagram({ config }: { config: Record<string, unknown> }) {
  const ttl = String(config.token_ttl_secs ?? "3600");
  const hb = String(config.signaling_heartbeat_interval_secs ?? "30");

  const kX = 80;   // KS (left — backend dependency)
  const aX = 280;  // AIS (center)
  const cX = 480;  // Client (right — flows toward Signaling)
  const sX = 640;  // Signaling (far right, no lifeline — just destination)
  const headY = 8;
  const lifeTop = 56;
  const lifeBot = 280;

  return (
    <svg
      viewBox="0 0 720 292"
      className="max-w-3xl mx-auto"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <marker id="ais-ab" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#3b82f6" />
        </marker>
        <marker id="ais-ag" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#10b981" />
        </marker>
        <marker id="ais-ap" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#8b5cf6" />
        </marker>
        <marker id="ais-ao" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#d97706" />
        </marker>
      </defs>

      {/* Column heads */}
      <rect x={kX - 50} y={headY} width="100" height="40" rx="8" fill="#fef3c7" stroke="#d97706" strokeWidth="1.5" />
      <text x={kX} y={headY + 18} textAnchor="middle" fontSize="11" fontWeight="600" fill="#92400e">KS</text>
      <text x={kX} y={headY + 31} textAnchor="middle" fontSize="8" fill="#d97706">Key Server</text>

      <rect x={aX - 50} y={headY} width="100" height="40" rx="8" fill="#e0e7ff" stroke="#6366f1" strokeWidth="1.5" />
      <text x={aX} y={headY + 18} textAnchor="middle" fontSize="11" fontWeight="600" fill="#3730a3">AIS</text>
      <text x={aX} y={headY + 31} textAnchor="middle" fontSize="8" fill="#6366f1">Identity Issuer</text>

      <rect x={cX - 50} y={headY} width="100" height="40" rx="8" fill="#dbeafe" stroke="#3b82f6" strokeWidth="1.5" />
      <text x={cX} y={headY + 18} textAnchor="middle" fontSize="11" fontWeight="600" fill="#1e40af">Client</text>
      <text x={cX} y={headY + 31} textAnchor="middle" fontSize="8" fill="#3b82f6">WebRTC peer</text>

      {/* Signaling — destination only (no lifeline) */}
      <rect x={sX - 50} y={headY} width="100" height="40" rx="8" fill="#d1fae5" stroke="#10b981" strokeWidth="1.5" strokeDasharray="4 2" />
      <text x={sX} y={headY + 18} textAnchor="middle" fontSize="11" fontWeight="600" fill="#065f46">Signaling</text>
      <text x={sX} y={headY + 31} textAnchor="middle" fontSize="8" fill="#10b981">WebSocket</text>

      {/* Lifelines */}
      <line x1={kX} y1={lifeTop} x2={kX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />
      <line x1={aX} y1={lifeTop} x2={aX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />
      <line x1={cX} y1={lifeTop} x2={cX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />
      <line x1={sX} y1={lifeTop} x2={sX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />

      {/* 1. AIS fetches public key from KS */}
      <line x1={aX - 4} y1={74} x2={kX + 4} y2={74} stroke="#d97706" strokeWidth="1.5" markerEnd="url(#ais-ao)" />
      <text x={(kX + aX) / 2} y={68} textAnchor="middle" fontSize="9" fontWeight="600" fill="#d97706">GetPublicKey</text>

      <line x1={kX + 4} y1={94} x2={aX - 4} y2={94} stroke="#d97706" strokeWidth="1.5" markerEnd="url(#ais-ao)" />
      <text x={(kX + aX) / 2} y={88} textAnchor="middle" fontSize="8" fill="#d97706">EC public key + key_id</text>

      {/* Cache note */}
      <rect x={aX - 86} y={100} width="80" height="18" rx="3" fill="#fef3c7" stroke="#fcd34d" strokeWidth="0.8" />
      <text x={aX - 46} y={112} textAnchor="middle" fontSize="7" fill="#92400e">cache locally</text>

      {/* 2. Client registers */}
      <line x1={cX - 4} y1={136} x2={aX + 4} y2={136} stroke="#3b82f6" strokeWidth="1.5" markerEnd="url(#ais-ab)" />
      <text x={(aX + cX) / 2} y={130} textAnchor="middle" fontSize="9" fontWeight="600" fill="#3b82f6">POST /register</text>

      {/* 3. AIS issues credential */}
      <rect x={aX - 44} y={148} width="88" height="36" rx="4" fill="#f5f3ff" stroke="#8b5cf6" strokeWidth="0.8" />
      <text x={aX} y={160} textAnchor="middle" fontSize="7" fontWeight="600" fill="#6d28d9">Generate ActrId</text>
      <text x={aX} y={170} textAnchor="middle" fontSize="7" fill="#7c3aed">ECIES encrypt</text>
      <text x={aX} y={180} textAnchor="middle" fontSize="7" fill="#7c3aed">with KS pubkey</text>

      {/* 4. Response to client */}
      <line x1={aX + 4} y1={198} x2={cX - 4} y2={198} stroke="#10b981" strokeWidth="1.5" markerEnd="url(#ais-ag)" />
      <text x={(aX + cX) / 2} y={192} textAnchor="middle" fontSize="9" fontWeight="600" fill="#10b981">Credential</text>
      <text x={(aX + cX) / 2} y={210} textAnchor="middle" fontSize="8" fill="#6b7280">ActrId + token + PSK</text>

      {/* 5. Client connects to Signaling */}
      <line x1={cX + 4} y1={240} x2={sX - 4} y2={240} stroke="#10b981" strokeWidth="1.5" markerEnd="url(#ais-ag)" />
      <text x={(cX + sX) / 2} y={234} textAnchor="middle" fontSize="9" fontWeight="600" fill="#10b981">Connect</text>
      <text x={(cX + sX) / 2} y={252} textAnchor="middle" fontSize="8" fill="#6b7280">with credential</text>

      {/* Heartbeat */}
      <line x1={cX + 4} y1={270} x2={sX - 4} y2={270} stroke="#10b981" strokeWidth="1" strokeDasharray="3 2" markerEnd="url(#ais-ag)" />
      <text x={(cX + sX) / 2} y={282} textAnchor="middle" fontSize="7" fill="#10b981">heartbeat every {hb}s</text>

      {/* TTL annotation */}
      <rect x={kX - 50} y={230} width="100" height="30" rx="4" fill="#f8fafc" stroke="#e2e8f0" strokeWidth="0.8" />
      <text x={kX} y={243} textAnchor="middle" fontSize="7" fill="#64748b">token_ttl: {ttl}s</text>
      <text x={kX} y={253} textAnchor="middle" fontSize="7" fill="#64748b">heartbeat: {hb}s</text>
    </svg>
  );
}

export function AisService() {
  const [data, setData] = useState<ServiceDetail | null>(null);
  const [keys, setKeys] = useState<KeyEntry[]>([]);
  const [error, setError] = useState("");

  const fetchData = useCallback(async () => {
    try {
      const [d, k] = await Promise.all([
        api.getServiceDetail("ais"),
        api.getAisKeys().catch(() => ({ keys: [] })),
      ]);
      setData(d);
      setKeys(k.keys);
      setError("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load");
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  if (error && !data) {
    return (
      <div className="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-700">
        {error}
      </div>
    );
  }

  if (!data) {
    return <div className="text-sm text-gray-500">Loading...</div>;
  }

  return (
    <ServicePageLayout
      title="AIS Service"
      description="Actor Identity Service — issues signed credentials for WebRTC actors"
    >
      <StatusSection
        enabled={data.enabled}
        healthy={data.status?.is_healthy}
        disabledHint={<>This service is not enabled. Set the AIS bit (bit 3) in the <code>enable</code> bitmask to activate it.</>}
      />

      {data.enabled && <ServiceMetrics status={data.status} storageKey="ais" />}

      {data.config && (
        <HowItWorks storageKey="ais">
          <p className="text-xs text-gray-500 mb-4">
            AIS issues identity credentials for WebRTC peers. On startup it fetches an EC public key
            from KS and caches it locally. When a client registers, AIS generates a unique ActrId,
            encrypts it with the KS public key via ECIES, and returns the credential along with a PSK.
            The client then uses this credential to connect to the Signaling server.
          </p>
          <AisDiagram config={data.config} />

          <div className="mt-5 space-y-2 text-xs text-gray-500 border-t border-gray-100 pt-4">
            <p className="font-semibold text-gray-600">Key concepts</p>
            <ul className="list-disc pl-4 space-y-1.5">
              <li>
                <strong className="text-gray-600">token_ttl_secs</strong> controls how long the issued
                credential is valid. Clients must re-register after expiry.
              </li>
              <li>
                <strong className="text-gray-600">signaling_heartbeat_interval_secs</strong> is sent to
                the client so it knows how often to ping the Signaling server to maintain its WebSocket connection.
              </li>
              <li>
                AIS caches the KS public key locally and refreshes it in the background.
                If KS is temporarily unavailable, AIS can continue serving from cache.
              </li>
            </ul>
          </div>
        </HowItWorks>
      )}

      {data.config_fields && (
        <ConfigSection storageKey="ais" fields={data.config_fields} onRefresh={fetchData} />
      )}

      <CollapsibleCard storageKey="ais_keys" title="Current Key">
        {keys.length === 0 ? (
          <p className="text-sm text-gray-500">No key cached yet</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-gray-200 text-left">
                  <th className="pb-2 pr-4 font-medium text-gray-500">Key ID</th>
                  <th className="pb-2 pr-4 font-medium text-gray-500">PK Size</th>
                  <th className="pb-2 pr-4 font-medium text-gray-500">Fetched At</th>
                  <th className="pb-2 pr-4 font-medium text-gray-500">Expires At</th>
                  <th className="pb-2 pr-4 font-medium text-gray-500">Tolerance</th>
                  <th className="pb-2 font-medium text-gray-500">Status</th>
                </tr>
              </thead>
              <tbody>
                {keys.map((k) => (
                  <tr key={k.key_id} className="border-b border-gray-100 last:border-0">
                    <td className="py-2 pr-4 font-mono">{k.key_id}</td>
                    <td className="py-2 pr-4">{k.pk_size} bytes</td>
                    <td className="py-2 pr-4 font-mono text-xs">
                      {k.fetched_at ? new Date(k.fetched_at * 1000).toLocaleString() : "—"}
                    </td>
                    <td className="py-2 pr-4 font-mono text-xs">
                      {new Date(k.expires_at * 1000).toLocaleString()}
                    </td>
                    <td className="py-2 pr-4">{k.tolerance_seconds != null ? `${k.tolerance_seconds}s` : "—"}</td>
                    <td className="py-2">
                      {k.is_expired ? (
                        <span className="inline-flex items-center rounded-full bg-red-100 px-2 py-0.5 text-xs font-medium text-red-700">
                          Expired
                        </span>
                      ) : (
                        <span className="inline-flex items-center rounded-full bg-green-100 px-2 py-0.5 text-xs font-medium text-green-700">
                          Valid
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CollapsibleCard>
    </ServicePageLayout>
  );
}
