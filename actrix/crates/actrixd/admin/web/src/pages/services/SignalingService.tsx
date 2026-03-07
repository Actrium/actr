import { useEffect, useState, useCallback } from "react";
import { api, type ServiceDetail } from "../../lib/api";
import { ServicePageLayout, ConfigSection, StatusSection } from "../../components/layout/ServicePageLayout";
import { HowItWorks } from "../../components/ui/HowItWorks";
import { ServiceMetrics } from "./shared";

function SignalingDiagram({ config }: { config: Record<string, unknown> }) {
  const wsPath = String(config.ws_path ?? "/signaling");

  /* ── layout ── */
  const aX = 80;   // Peer A
  const sX = 310;  // Signaling Server
  const bX = 540;  // Peer B
  const headY = 8;
  const lifeTop = 56;
  const lifeBot = 310;

  return (
    <svg
      viewBox="0 0 620 324"
      className="max-w-2xl mx-auto"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <marker id="sig-ar" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#3b82f6" />
        </marker>
        <marker id="sig-ag" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#10b981" />
        </marker>
        <marker id="sig-ao" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#d97706" />
        </marker>
        <marker id="sig-ap" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#8b5cf6" />
        </marker>
      </defs>

      {/* ══ Column heads ══════════════════════════════ */}
      <rect x={aX - 50} y={headY} width="100" height="40" rx="8" fill="#dbeafe" stroke="#3b82f6" strokeWidth="1.5" />
      <text x={aX} y={headY + 18} textAnchor="middle" fontSize="11" fontWeight="600" fill="#1e40af">Peer A</text>
      <text x={aX} y={headY + 31} textAnchor="middle" fontSize="8" fill="#3b82f6">caller</text>

      <rect x={sX - 60} y={headY} width="120" height="40" rx="8" fill="#e0e7ff" stroke="#6366f1" strokeWidth="1.5" />
      <text x={sX} y={headY + 18} textAnchor="middle" fontSize="11" fontWeight="600" fill="#3730a3">Signaling</text>
      <text x={sX} y={headY + 31} textAnchor="middle" fontSize="8" fill="#6366f1">ws://*{wsPath}</text>

      <rect x={bX - 50} y={headY} width="100" height="40" rx="8" fill="#d1fae5" stroke="#10b981" strokeWidth="1.5" />
      <text x={bX} y={headY + 18} textAnchor="middle" fontSize="11" fontWeight="600" fill="#065f46">Peer B</text>
      <text x={bX} y={headY + 31} textAnchor="middle" fontSize="8" fill="#10b981">callee</text>

      {/* ══ Lifelines ═════════════════════════════════ */}
      <line x1={aX} y1={lifeTop} x2={aX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />
      <line x1={sX} y1={lifeTop} x2={sX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />
      <line x1={bX} y1={lifeTop} x2={bX} y2={lifeBot} stroke="#cbd5e1" strokeWidth="1" strokeDasharray="4 3" />

      {/* ── 1. Register ───────────────────────────── */}
      <line x1={aX + 4} y1={74} x2={sX - 4} y2={74} stroke="#3b82f6" strokeWidth="1.5" markerEnd="url(#sig-ar)" />
      <text x={(aX + sX) / 2} y={68} textAnchor="middle" fontSize="9" fontWeight="600" fill="#3b82f6">Register</text>

      <line x1={bX - 4} y1={92} x2={sX + 4} y2={92} stroke="#10b981" strokeWidth="1.5" markerEnd="url(#sig-ag)" />
      <text x={(sX + bX) / 2} y={86} textAnchor="middle" fontSize="9" fontWeight="600" fill="#10b981">Register</text>

      {/* ── 2. Discovery ──────────────────────────── */}
      <line x1={aX + 4} y1={118} x2={sX - 4} y2={118} stroke="#d97706" strokeWidth="1.5" markerEnd="url(#sig-ao)" />
      <text x={(aX + sX) / 2} y={112} textAnchor="middle" fontSize="9" fontWeight="600" fill="#d97706">Discovery</text>

      <line x1={sX - 4} y1={136} x2={aX + 4} y2={136} stroke="#d97706" strokeWidth="1.5" markerEnd="url(#sig-ao)" />
      <text x={(aX + sX) / 2} y={130} textAnchor="middle" fontSize="8" fill="#d97706">found Peer B</text>

      {/* ── 3. SDP Offer/Answer ────────────────────── */}
      <line x1={aX + 4} y1={162} x2={sX - 4} y2={162} stroke="#8b5cf6" strokeWidth="1.5" markerEnd="url(#sig-ap)" />
      <line x1={sX + 4} y1={162} x2={bX - 4} y2={162} stroke="#8b5cf6" strokeWidth="1.5" markerEnd="url(#sig-ap)" />
      <text x={sX} y={156} textAnchor="middle" fontSize="9" fontWeight="600" fill="#8b5cf6">SDP Offer</text>

      <line x1={bX - 4} y1={186} x2={sX + 4} y2={186} stroke="#8b5cf6" strokeWidth="1.5" markerEnd="url(#sig-ap)" />
      <line x1={sX - 4} y1={186} x2={aX + 4} y2={186} stroke="#8b5cf6" strokeWidth="1.5" markerEnd="url(#sig-ap)" />
      <text x={sX} y={180} textAnchor="middle" fontSize="9" fontWeight="600" fill="#8b5cf6">SDP Answer</text>

      {/* ── 4. ICE candidates ─────────────────────── */}
      <line x1={aX + 4} y1={212} x2={sX - 4} y2={212} stroke="#8b5cf6" strokeWidth="1.2" strokeDasharray="5 3" markerEnd="url(#sig-ap)" />
      <line x1={sX + 4} y1={212} x2={bX - 4} y2={212} stroke="#8b5cf6" strokeWidth="1.2" strokeDasharray="5 3" markerEnd="url(#sig-ap)" />
      <text x={sX} y={206} textAnchor="middle" fontSize="8" fill="#8b5cf6">ICE candidates</text>

      <line x1={bX - 4} y1={230} x2={sX + 4} y2={230} stroke="#8b5cf6" strokeWidth="1.2" strokeDasharray="5 3" markerEnd="url(#sig-ap)" />
      <line x1={sX - 4} y1={230} x2={aX + 4} y2={230} stroke="#8b5cf6" strokeWidth="1.2" strokeDasharray="5 3" markerEnd="url(#sig-ap)" />
      <text x={sX} y={224} textAnchor="middle" fontSize="8" fill="#8b5cf6">ICE candidates</text>

      {/* ── 5. P2P established ────────────────────── */}
      <line x1={aX + 4} y1={268} x2={bX - 4} y2={268} stroke="#10b981" strokeWidth="2" />
      <rect x={sX - 55} y={258} width="110" height="18" rx="4" fill="#ecfdf5" />
      <text x={sX} y={270} textAnchor="middle" fontSize="9" fontWeight="700" fill="#065f46">P2P established</text>
    </svg>
  );
}

export function SignalingService() {
  const [data, setData] = useState<ServiceDetail | null>(null);
  const [error, setError] = useState("");

  const fetchData = useCallback(async () => {
    try {
      const d = await api.getServiceDetail("signaling");
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
      title="Signaling Service"
      description="WebSocket signaling server for WebRTC session negotiation"
    >
      <StatusSection
        enabled={data.enabled}
        healthy={data.status?.is_healthy}
        disabledHint={<>This service is not enabled. Set the Signaling bit (bit 0) in the <code>enable</code> bitmask to activate it.</>}
      />

      {data.enabled && <ServiceMetrics status={data.status} storageKey="signaling" />}

      {data.config && (
        <HowItWorks storageKey="signaling">
          <p className="text-xs text-gray-500 mb-4">
            Peers connect via WebSocket, register their identity, discover each other,
            then exchange SDP offers/answers and ICE candidates through the server.
            Once negotiation completes, a direct P2P connection is established and
            the signaling channel becomes idle.
          </p>
          <SignalingDiagram config={data.config} />
        </HowItWorks>
      )}

      {data.config_fields && (
        <ConfigSection storageKey="signaling" fields={data.config_fields} onRefresh={fetchData} />
      )}
    </ServicePageLayout>
  );
}
