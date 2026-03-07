import { useEffect, useState, useCallback } from "react";
import { api, type ServiceDetail, type KeyEntry } from "../../lib/api";
import { ServicePageLayout, ConfigSection, StatusSection } from "../../components/layout/ServicePageLayout";
import { HowItWorks } from "../../components/ui/HowItWorks";
import { ServiceMetrics } from "./shared";
import { CollapsibleCard } from "../../components/ui/CollapsibleCard";

function KsLifecycleDiagram({ config }: { config: Record<string, unknown> }) {
  const ttl = Number(config.key_ttl_seconds ?? 3600);
  const tolerance = Number(config.tolerance_seconds ?? 300);

  // Timeline proportions
  const totalSpan = ttl + tolerance;
  const activeRatio = ttl / totalSpan;

  // Layout
  const padL = 40;
  const barW = 520;
  const barY = 198;
  const barH = 28;
  const activeW = Math.round(barW * activeRatio);
  const toleranceW = barW - activeW;

  // Flow section layout
  const ksX = 300;  // KS center

  return (
    <svg
      viewBox="0 0 600 340"
      className="max-w-2xl mx-auto"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <marker id="ks-ab" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#3b82f6" />
        </marker>
        <marker id="ks-ag" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#10b981" />
        </marker>
        <marker id="ks-ao" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#d97706" />
        </marker>
        <marker id="ks-ap" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#8b5cf6" />
        </marker>
        <marker id="ks-ar" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#9ca3af" />
        </marker>
      </defs>

      {/* ═══ Two API flows ═══ */}

      {/* KS center box */}
      <rect x={ksX - 56} y="26" width="112" height="48" rx="8" fill="#fef3c7" stroke="#d97706" strokeWidth="1.5" />
      <text x={ksX} y="44" textAnchor="middle" fontSize="11" fontWeight="600" fill="#92400e">KS</text>
      <text x={ksX} y="57" textAnchor="middle" fontSize="8" fill="#d97706">Cluster Key Store</text>
      <text x={ksX} y="68" textAnchor="middle" fontSize="6.5" fontWeight="500" fill="#b45309" fontStyle="italic">cluster-private</text>

      {/* SQLite box below KS */}
      <rect x={ksX - 36} y="90" width="72" height="24" rx="4" fill="#f1f5f9" stroke="#94a3b8" strokeWidth="1" />
      <text x={ksX} y="106" textAnchor="middle" fontSize="8" fontWeight="600" fill="#475569">SQLite</text>
      <line x1={ksX} y1="74" x2={ksX} y2="90" stroke="#94a3b8" strokeWidth="1" markerEnd="url(#ks-ar)" />

      {/* ── Left: GenerateKey (AIS → KS) ── */}
      <rect x="16" y="34" width="100" height="32" rx="6" fill="#e0e7ff" stroke="#6366f1" strokeWidth="1.2" />
      <text x="66" y="48" textAnchor="middle" fontSize="10" fontWeight="600" fill="#3730a3">AIS</text>
      <text x="66" y="59" textAnchor="middle" fontSize="7" fill="#6366f1">Issuer</text>

      {/* AIS → KS */}
      <line x1="116" y1="44" x2={ksX - 58} y2="44" stroke="#3b82f6" strokeWidth="1.5" markerEnd="url(#ks-ab)" />
      <text x="178" y="38" textAnchor="middle" fontSize="8" fontWeight="600" fill="#3b82f6">GenerateKey</text>

      {/* KS → AIS (response) */}
      <line x1={ksX - 58} y1="58" x2="116" y2="58" stroke="#10b981" strokeWidth="1.5" markerEnd="url(#ks-ag)" />
      <text x="178" y="56" textAnchor="middle" fontSize="7" fill="#10b981">public_key + key_id</text>

      {/* ── Right: GetSecretKey (Verifier logic → KS) ── */}
      <rect x="476" y="28" width="116" height="42" rx="6" fill="#f5f3ff" stroke="#8b5cf6" strokeWidth="1.2" strokeDasharray="4 2" />
      <text x="534" y="44" textAnchor="middle" fontSize="9" fontWeight="600" fill="#5b21b6">Verifier logic</text>
      <text x="534" y="56" textAnchor="middle" fontSize="7" fill="#8b5cf6">in Signaling + TURN</text>
      <text x="534" y="65" textAnchor="middle" fontSize="6" fill="#a78bfa">AIdCredentialValidator</text>

      {/* Verifier → KS */}
      <line x1="476" y1="44" x2={ksX + 58} y2="44" stroke="#8b5cf6" strokeWidth="1.5" markerEnd="url(#ks-ap)" />
      <text x="422" y="38" textAnchor="middle" fontSize="8" fontWeight="600" fill="#8b5cf6">GetSecretKey</text>

      {/* KS → Verifier (response) */}
      <line x1={ksX + 58} y1="58" x2="476" y2="58" stroke="#8b5cf6" strokeWidth="1.5" markerEnd="url(#ks-ap)" />
      <text x="422" y="56" textAnchor="middle" fontSize="7" fill="#8b5cf6">secret_key (for decrypt)</text>

      {/* Auth note */}
      <text x={ksX} y="124" textAnchor="middle" fontSize="7" fill="#94a3b8">
        All requests authenticated via nonce-auth (HMAC-SHA256 + replay protection)
      </text>

      {/* ═══ Divider ═══ */}
      <line x1="16" y1="140" x2="584" y2="140" stroke="#e2e8f0" strokeWidth="1" />

      {/* ═══ Key Lifecycle ═══ */}
      <text x={padL} y="160" fontSize="10" fontWeight="600" fill="#475569">Key Lifecycle</text>
      <text x={padL} y="174" fontSize="8" fill="#94a3b8">
        total validity = {ttl}s active + {tolerance}s tolerance
      </text>

      {/* Timeline bar — Active */}
      <rect x={padL} y={barY} width={activeW} height={barH} rx="4"
        fill="#dcfce7" stroke="#22c55e" strokeWidth="1.2" />
      <text x={padL + activeW / 2} y={barY + 12} textAnchor="middle"
        fontSize="9" fontWeight="600" fill="#166534">
        Active ({ttl}s)
      </text>
      <text x={padL + activeW / 2} y={barY + 23} textAnchor="middle"
        fontSize="7" fill="#16a34a">
        encrypt + decrypt
      </text>

      {/* Timeline bar — Tolerance */}
      <rect x={padL + activeW} y={barY} width={toleranceW} height={barH} rx="4"
        fill="#fef3c7" stroke="#f59e0b" strokeWidth="1.2" />
      <text x={padL + activeW + toleranceW / 2} y={barY + 12} textAnchor="middle"
        fontSize="9" fontWeight="600" fill="#92400e">
        Tolerance ({tolerance}s)
      </text>
      <text x={padL + activeW + toleranceW / 2} y={barY + 23} textAnchor="middle"
        fontSize="7" fill="#b45309">
        decrypt only
      </text>

      {/* Time markers */}
      <line x1={padL} y1={barY + barH + 4} x2={padL} y2={barY + barH + 14}
        stroke="#64748b" strokeWidth="1" />
      <text x={padL} y={barY + barH + 24} textAnchor="middle" fontSize="8" fill="#64748b">
        Created
      </text>

      <line x1={padL + activeW} y1={barY + barH + 4} x2={padL + activeW} y2={barY + barH + 14}
        stroke="#64748b" strokeWidth="1" />
      <text x={padL + activeW} y={barY + barH + 24} textAnchor="middle" fontSize="8" fill="#64748b">
        Expires
      </text>

      <line x1={padL + barW} y1={barY + barH + 4} x2={padL + barW} y2={barY + barH + 14}
        stroke="#64748b" strokeWidth="1" />
      <text x={padL + barW} y={barY + barH + 24} textAnchor="middle" fontSize="8" fill="#64748b">
        Cleanup
      </text>

      {/* Legend */}
      <rect x={padL} y="268" width="12" height="12" rx="2" fill="#dcfce7" stroke="#22c55e" strokeWidth="0.8" />
      <text x={padL + 18} y="278" fontSize="8" fill="#475569">
        GenerateKey returns public key; GetSecretKey returns private key
      </text>

      <rect x={padL} y="286" width="12" height="12" rx="2" fill="#fef3c7" stroke="#f59e0b" strokeWidth="0.8" />
      <text x={padL + 18} y="296" fontSize="8" fill="#475569">
        GetSecretKey still works (verify old credentials); GenerateKey uses new key
      </text>

      <rect x={padL} y="304" width="12" height="12" rx="2" fill="#fee2e2" stroke="#f87171" strokeWidth="0.8" />
      <text x={padL + 18} y="314" fontSize="8" fill="#475569">
        Key removed from DB (lazy cleanup every 100 requests, min 10 keys)
      </text>
    </svg>
  );
}

export function KsService() {
  const [data, setData] = useState<ServiceDetail | null>(null);
  const [keys, setKeys] = useState<KeyEntry[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [error, setError] = useState("");
  const [cleaning, setCleaning] = useState(false);
  const [cleanupMsg, setCleanupMsg] = useState("");

  const fetchData = useCallback(async () => {
    try {
      const [d, k] = await Promise.all([
        api.getServiceDetail("ks"),
        api.getKsKeys().catch(() => ({ keys: [], total_count: 0 })),
      ]);
      setData(d);
      setKeys(k.keys);
      setTotalCount(k.total_count);
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
      title="KS Service"
      description="Cluster Key Store — generates and manages EC key pairs for credential encryption, shared across the Actrix cluster and not exposed to clients"
    >
      <StatusSection
        enabled={data.enabled}
        healthy={data.status?.is_healthy}
        disabledHint={<>This service is not enabled. Set the KS bit (bit 4) in the <code>enable</code> bitmask to activate it.</>}
      />

      {data.enabled && <ServiceMetrics status={data.status} storageKey="ks" />}

      {data.config && (
        <HowItWorks storageKey="ks">
          <p className="text-xs text-gray-500 mb-4">
            KS exposes two core APIs: <strong>GenerateKey</strong> creates an EC key pair, stores it
            in SQLite, and returns the public key to AIS for encrypting credentials.
            <strong> GetSecretKey</strong> returns the private key to the verifier process
            so it can ECIES-decrypt and verify client credentials. Currently Signaling and TURN
            both act as verifiers via the shared <code className="text-[11px] bg-gray-100 px-1 rounded">AIdCredentialValidator</code>.
            Both APIs require nonce-auth (HMAC-SHA256 + one-time nonce).
            Keys follow a lifecycle: active for issuing + verifying, then a tolerance window
            for verification only, after which they are lazily cleaned up.
          </p>
          <KsLifecycleDiagram config={data.config} />

          <div className="mt-5 space-y-2 text-xs text-gray-500 border-t border-gray-100 pt-4">
            <p className="font-semibold text-gray-600">Key concepts</p>
            <ul className="list-disc pl-4 space-y-1.5">
              <li>
                <strong className="text-gray-600">Cluster-private</strong> — KS is an internal service
                shared across the Actrix cluster. It is never exposed to external clients; only
                other cluster services (AIS, Signaling, TURN) communicate with it via authenticated gRPC.
              </li>
              <li>
                <strong className="text-gray-600">GenerateKey</strong> — AIS calls this to get a fresh
                public key for ECIES encryption. The private key stays in KS's SQLite.
              </li>
              <li>
                <strong className="text-gray-600">GetSecretKey</strong> — the verifier process calls
                this with a <code className="text-[11px] bg-gray-100 px-1 rounded">key_id</code> to
                retrieve the private key and ECIES-decrypt a client's credential. Currently
                both Signaling (WebSocket auth) and TURN (relay auth) use this path via
                the shared <code className="text-[11px] bg-gray-100 px-1 rounded">AIdCredentialValidator</code>.
              </li>
              <li>
                <strong className="text-gray-600">key_ttl_seconds</strong> — how long a key pair is
                active. During this window both GenerateKey (for new credentials) and GetSecretKey
                (for verification) work. AIS refreshes before expiry.
              </li>
              <li>
                <strong className="text-gray-600">tolerance_seconds</strong> — grace period after
                expiry. Only GetSecretKey still works (verify old credentials); GenerateKey
                will use a newer key.
              </li>
            </ul>
          </div>
        </HowItWorks>
      )}

      {data.config_fields && (
        <ConfigSection storageKey="ks" fields={data.config_fields} onRefresh={fetchData} />
      )}

      <CollapsibleCard storageKey="ks_keys" title="Keys">
        <div className="flex items-center justify-between mb-3">
          <span className="text-xs text-gray-500">
            {keys.length} of {totalCount} total
          </span>
          <div className="flex items-center gap-2">
            {cleanupMsg && (
              <span className="text-xs text-green-600">{cleanupMsg}</span>
            )}
            <button
              onClick={async () => {
                setCleaning(true);
                setCleanupMsg("");
                try {
                  const r = await api.cleanupKsKeys();
                  setCleanupMsg(`Deleted ${r.deleted}, ${r.remaining} remaining`);
                  fetchData();
                } catch {
                  setCleanupMsg("Cleanup failed");
                } finally {
                  setCleaning(false);
                }
              }}
              disabled={cleaning}
              className="rounded-md border border-gray-300 px-2.5 py-1 text-xs font-medium text-gray-600 hover:bg-gray-50 disabled:opacity-50 transition-colors"
            >
              {cleaning ? "Cleaning..." : "Cleanup expired"}
            </button>
          </div>
        </div>
        {keys.length === 0 ? (
          <p className="text-sm text-gray-500">No keys found</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-gray-200 text-left">
                  <th className="pb-2 pr-4 font-medium text-gray-500">Key ID</th>
                  <th className="pb-2 pr-4 font-medium text-gray-500">PK Size</th>
                  <th className="pb-2 pr-4 font-medium text-gray-500">Created At</th>
                  <th className="pb-2 pr-4 font-medium text-gray-500">Expires At</th>
                  <th className="pb-2 font-medium text-gray-500">Status</th>
                </tr>
              </thead>
              <tbody>
                {keys.map((k) => (
                  <tr key={k.key_id} className="border-b border-gray-100 last:border-0">
                    <td className="py-2 pr-4 font-mono">{k.key_id}</td>
                    <td className="py-2 pr-4">{k.pk_size} bytes</td>
                    <td className="py-2 pr-4 font-mono text-xs">
                      {k.created_at ? new Date(k.created_at * 1000).toLocaleString() : "—"}
                    </td>
                    <td className="py-2 pr-4 font-mono text-xs">
                      {k.expires_at === 0
                        ? "Never"
                        : new Date(k.expires_at * 1000).toLocaleString()}
                    </td>
                    <td className="py-2">
                      {k.expires_at === 0 ? (
                        <span className="inline-flex items-center rounded-full bg-blue-100 px-2 py-0.5 text-xs font-medium text-blue-700">
                          Permanent
                        </span>
                      ) : k.is_expired ? (
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
