import { useEffect, useState, useCallback } from "react";
import { api, type ServiceDetail } from "../../lib/api";
import { ServicePageLayout, ConfigSection, StatusSection } from "../../components/layout/ServicePageLayout";
import { HowItWorks } from "../../components/ui/HowItWorks";
import { ServiceMetrics } from "./shared";

function StunDiagram({ config }: { config: Record<string, unknown> }) {
  const advIp = String(config.advertised_ip ?? "?");
  const advPort = String(config.advertised_port ?? "?");

  /* ── layout constants ── */
  const cX = 80;   // Client lifeline
  const nX = 280;  // NAT lifeline
  const sX = 500;  // Server lifeline
  const headY = 8;
  const lifeTop = 52;
  const lifeBot = 200;

  return (
    <svg
      viewBox="0 0 620 212"
      className="max-w-2xl mx-auto"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <marker id="s-ar" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#3b82f6" />
        </marker>
        <marker id="s-ag" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#10b981" />
        </marker>
      </defs>

      {/* ══ Column heads ══════════════════════════════ */}
      <rect x={cX - 50} y={headY} width="100" height="36" rx="8" fill="#dbeafe" stroke="#3b82f6" strokeWidth="1.5" />
      <text x={cX} y={headY + 22} textAnchor="middle" fontSize="11" fontWeight="600" fill="#1e40af">Client</text>

      <rect x={nX - 50} y={headY} width="100" height="36" rx="8" fill="#fef3c7" stroke="#d97706" strokeWidth="1.2" />
      <text x={nX} y={headY + 22} textAnchor="middle" fontSize="11" fontWeight="600" fill="#92400e">NAT</text>

      <rect x={sX - 60} y={headY} width="120" height="36" rx="8" fill="#e0e7ff" stroke="#6366f1" strokeWidth="1.5" />
      <text x={sX} y={headY + 15} textAnchor="middle" fontSize="11" fontWeight="600" fill="#3730a3">STUN Server</text>
      <text x={sX} y={headY + 28} textAnchor="middle" fontSize="8" fill="#6366f1">{advIp}:{advPort}</text>

      {/* ══ Lifelines ═════════════════════════════════ */}
      <line x1={cX} y1={lifeTop} x2={cX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />
      <line x1={nX} y1={lifeTop} x2={nX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />
      <line x1={sX} y1={lifeTop} x2={sX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />

      {/* ── Step 1: Client → NAT  (Binding Request) ── */}
      <line x1={cX + 4} y1={72} x2={nX - 4} y2={72} stroke="#3b82f6" strokeWidth="1.5" markerEnd="url(#s-ar)" />
      <text x={(cX + nX) / 2} y={66} textAnchor="middle" fontSize="9" fontWeight="600" fill="#3b82f6">Binding Request</text>
      <text x={(cX + nX) / 2} y={84} textAnchor="middle" fontSize="8" fill="#94a3b8">src 192.168.x.x:P</text>

      {/* ── Step 2: NAT → Server  (src rewritten) ──── */}
      <line x1={nX + 4} y1={104} x2={sX - 4} y2={104} stroke="#3b82f6" strokeWidth="1.5" markerEnd="url(#s-ar)" />
      <text x={(nX + sX) / 2} y={98} textAnchor="middle" fontSize="9" fontWeight="500" fill="#3b82f6">forwarded</text>
      <text x={(nX + sX) / 2} y={116} textAnchor="middle" fontSize="8" fontWeight="600" fill="#d97706">src rewritten → public IP:Q</text>

      {/* ── Note on server side ────────────────────── */}
      <rect x={sX + 6} y={126} width="105" height="22" rx="4" fill="#f8fafc" stroke="#cbd5e1" strokeWidth="0.8" />
      <text x={sX + 58} y={140} textAnchor="middle" fontSize="8" fill="#475569">reads src = IP:Q</text>

      {/* ── Step 3: Server → NAT → Client  (Response) ─ */}
      <line x1={sX - 4} y1={162} x2={nX + 4} y2={162} stroke="#10b981" strokeWidth="1.5" markerEnd="url(#s-ag)" />
      <line x1={nX - 4} y1={162} x2={cX + 4} y2={162} stroke="#10b981" strokeWidth="1.5" markerEnd="url(#s-ag)" />
      <text x={(cX + sX) / 2} y={156} textAnchor="middle" fontSize="9" fontWeight="600" fill="#10b981">Binding Response</text>

      {/* ── Result: Client learns its address ─────── */}
      <rect x={cX - 46} y={lifeBot - 22} width="92" height="14" rx="3" fill="#ecfdf5" />
      <text x={cX} y={lifeBot - 12} textAnchor="middle" fontSize="8" fill="#10b981">learned public IP:Q</text>
    </svg>
  );
}

export function StunService() {
  const [data, setData] = useState<ServiceDetail | null>(null);
  const [error, setError] = useState("");

  const fetchData = useCallback(async () => {
    try {
      const d = await api.getServiceDetail("stun");
      setData(d);
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
      title="STUN Service"
      description="Session Traversal Utilities for NAT — helps clients discover their public address"
    >
      <StatusSection
        enabled={data.enabled}
        healthy={data.status?.is_healthy}
        disabledHint={<>This service is not enabled. Set the STUN bit (bit 1) in the <code>enable</code> bitmask to activate it.</>}
      />

      {data.enabled && <ServiceMetrics status={data.status} storageKey="stun" />}

      {data.config && (
        <HowItWorks storageKey="stun">
          <p className="text-xs text-gray-500 mb-4">
            A client sends a Binding Request to the STUN server.
            The request passes through the client's NAT, which rewrites the source to a public IP:port.
            The server reads that source address and echoes it back in a XOR-MAPPED-ADDRESS attribute,
            so the client discovers its own public address.
          </p>
          <StunDiagram config={data.config} />

          <div className="mt-5 space-y-2 text-xs text-gray-500 border-t border-gray-100 pt-4">
            <p className="font-semibold text-gray-600">Deployment notes</p>
            <ul className="list-disc pl-4 space-y-1.5">
              <li>
                <strong className="text-gray-600">advertised_ip</strong> is the public address
                clients send Binding Requests to. It must be reachable from the internet.
              </li>
              <li>
                <strong className="text-gray-600">bind_ip</strong> is the local interface the process
                calls <code className="text-[11px] bg-gray-100 px-1 rounded">bind()</code> on.
                When it differs from advertised_ip (e.g. cloud VM with Elastic IP),
                the host needs 1:1 NAT to map the public address to the local interface.
              </li>
              <li>
                STUN is lightweight and stateless — each Binding transaction is a single
                UDP round-trip with no allocations or long-lived state.
              </li>
            </ul>
          </div>
        </HowItWorks>
      )}

      {data.config_fields && (
        <ConfigSection storageKey="stun" fields={data.config_fields} onRefresh={fetchData} />
      )}
    </ServicePageLayout>
  );
}
