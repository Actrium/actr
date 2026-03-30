import { useEffect, useState, useCallback } from "react";
import { api, type ServiceDetail, type KeyEntry, type ServiceStatus } from "../../lib/api";
import { ServicePageLayout, ConfigSection, StatusSection } from "../../components/layout/ServicePageLayout";
import { HowItWorks } from "../../components/ui/HowItWorks";
import { ServiceMetrics } from "./shared";
import { CollapsibleCard } from "../../components/ui/CollapsibleCard";

const sectionTitleStyle = {
  fontFamily: 'ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
  fontSize: "15.5px",
  fontWeight: 700,
  lineHeight: 1.2,
  color: "#334155",
} as const;

const sectionSubtitleStyle = {
  fontFamily: 'ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
  fontSize: "11.5px",
  lineHeight: 1.45,
  color: "#64748b",
} as const;

function IssuanceDiagram() {
  const totalH = 506;
  const panelX = 10;
  const panelW = 880;

  const nodeH = 62;
  const nodeY = 60;
  const storageY = nodeY + nodeH + 18;
  const rotationY = storageY + 54;

  const aPanelH = 264;
  const bPanelY = aPanelH + 20;
  const bPanelH = 210;

  return (
    <svg
      viewBox={`0 0 900 ${totalH}`}
      role="img"
      aria-label="Credential Issuance Flow"
      className="block h-auto w-full"
      preserveAspectRatio="xMidYMin meet"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <filter id="sgn-shadow" x="-20%" y="-20%" width="140%" height="140%">
          <feDropShadow dx="0" dy="4" stdDeviation="8" floodColor="#0f172a" floodOpacity="0.07" />
        </filter>
        <marker id="sgn-ag" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#10b981" />
        </marker>
        <marker id="sgn-ao" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#d97706" />
        </marker>
        <marker id="sgn-ap" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#8b5cf6" />
        </marker>
        <marker id="sgn-at" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#f97316" />
        </marker>
        <marker id="sgn-ak" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#22c55e" />
        </marker>
        <marker id="sgn-ar" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#9ca3af" />
        </marker>
        <marker id="sgn-ab" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#3b82f6" />
        </marker>
      </defs>
      <style>{`
        text { font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
        .sgn-muted { fill: #64748b; font-size: 11.5px; }
        .sgn-chip-title { font-size: 10.5px; font-weight: 700; }
        .sgn-chip-caption, .sgn-arrow-label { font-size: 9.4px; }
        .sgn-chip-caption { fill: #64748b; }
        .sgn-node-title { font-size: 12.5px; font-weight: 700; }
        .sgn-node-subtitle { font-size: 9px; }
        .sgn-detail-text { font-size: 9px; }
        .sgn-panel { fill: #ffffff; stroke-width: 1.15px; }
        .sgn-label-chip { fill: #ffffff; stroke: #e2e8f0; stroke-width: 1; }
      `}</style>

      {/* ── Sub-panel A: Key Lifecycle ── */}
      <rect x={panelX} y="10" width={panelW} height={aPanelH} rx="16" className="sgn-panel" stroke="#c7d2fe" />
      <rect x="26" y="24" width="188" height="24" rx="12" fill="#eef2ff" stroke="#a5b4fc" strokeWidth="1" />
      <text x="120" y="40" textAnchor="middle" className="sgn-chip-title" fill="#4338ca">A · Key Lifecycle</text>
      <text x="228" y="40" className="sgn-chip-caption">background refresh</text>

      <rect x="36" y={nodeY} width="136" height={nodeH} rx="14" fill="#eef2ff" stroke="#6366f1" strokeWidth="1.6" filter="url(#sgn-shadow)" />
      <text x="104" y={nodeY + 24} textAnchor="middle" className="sgn-node-title" fill="#3730a3">AIS</text>
      <text x="104" y={nodeY + 42} textAnchor="middle" className="sgn-node-subtitle" fill="#6366f1">refresh controller</text>

      <rect x="240" y={nodeY} width="154" height={nodeH} rx="14" fill="#fffbeb" stroke="#d97706" strokeWidth="1.6" filter="url(#sgn-shadow)" />
      <text x="317" y={nodeY + 24} textAnchor="middle" className="sgn-node-title" fill="#92400e">Signer (KS)</text>
      <text x="317" y={nodeY + 42} textAnchor="middle" className="sgn-node-subtitle" fill="#d97706">guards private keys</text>

      <line x1="317" y1={nodeY + nodeH} x2="317" y2={storageY} stroke="#f59e0b" strokeWidth="1.2" strokeDasharray="4 3" markerEnd="url(#sgn-at)" />
      <rect x="244" y={storageY} width="146" height="30" rx="10" fill="#fff7ed" stroke="#fdba74" strokeWidth="1" />
      <text x="317" y={storageY + 12} textAnchor="middle" fontSize="8.8" fontWeight="700" fill="#9a3412">Signer storage backend</text>
      <text x="317" y={storageY + 23} textAnchor="middle" fontSize="7.8" fill="#c2410c">private keys never leave process</text>

      <rect x="472" y={nodeY} width="146" height={nodeH} rx="14" fill="#f8fafc" stroke="#94a3b8" strokeWidth="1.3" filter="url(#sgn-shadow)" />
      <text x="545" y={nodeY + 24} textAnchor="middle" fontSize="11.2" fontWeight="600" fill="#334155">ais_keys.db</text>
      <text x="545" y={nodeY + 42} textAnchor="middle" className="sgn-node-subtitle" fill="#64748b">AIS private verifier cache</text>

      <rect x="642" y={nodeY} width="166" height={nodeH} rx="14" fill="#f0fdf4" stroke="#22c55e" strokeWidth="1.3" filter="url(#sgn-shadow)" />
      <text x="725" y={nodeY + 24} textAnchor="middle" fontSize="11.2" fontWeight="600" fill="#166534">signaling_key_cache.db</text>
      <text x="725" y={nodeY + 42} textAnchor="middle" className="sgn-node-subtitle" fill="#16a34a">shared verifier store</text>

      <line x1="172" y1={nodeY + 24} x2="240" y2={nodeY + 24} stroke="#6366f1" strokeWidth="1.6" markerEnd="url(#sgn-ap)" />
      <rect x="146" y={nodeY - 18} width="120" height="16" rx="8" className="sgn-label-chip" />
      <text x="206" y={nodeY - 6} textAnchor="middle" className="sgn-arrow-label" fill="#4338ca">GenerateSigningKey RPC</text>

      <line x1="240" y1={nodeY + 42} x2="172" y2={nodeY + 42} stroke="#10b981" strokeWidth="1.5" markerEnd="url(#sgn-ag)" />
      <rect x="142" y={nodeY + 52} width="128" height="16" rx="8" className="sgn-label-chip" />
      <text x="206" y={nodeY + 64} textAnchor="middle" className="sgn-arrow-label" fill="#059669">key_id + verifier + expiry</text>

      <line x1="394" y1={nodeY + 24} x2="472" y2={nodeY + 24} stroke="#94a3b8" strokeWidth="1.4" markerEnd="url(#sgn-ar)" />
      <rect x="373" y={nodeY - 18} width="120" height="16" rx="8" className="sgn-label-chip" />
      <text x="433" y={nodeY - 6} textAnchor="middle" className="sgn-arrow-label" fill="#64748b">store active verifier</text>

      <path d={`M 394 ${nodeY + 46} Q 520 ${storageY + 26} 642 ${nodeY + 46}`} fill="none" stroke="#22c55e" strokeWidth="1.4" markerEnd="url(#sgn-ak)" />
      <rect x="444" y={storageY + 12} width="148" height="16" rx="8" className="sgn-label-chip" />
      <text x="518" y={storageY + 24} textAnchor="middle" className="sgn-arrow-label" fill="#16a34a">propagate to shared cache</text>

      <rect x="36" y={rotationY} width="828" height="40" rx="12" fill="#fff7ed" stroke="#fed7aa" strokeWidth="1" />
      <text x="450" y={rotationY + 16} textAnchor="middle" fontSize="9.6" fontWeight="700" fill="#92400e">⟳ Background rotation</text>
      <text x="450" y={rotationY + 29} textAnchor="middle" className="sgn-detail-text" fill="#b45309">Check every 10 min: rotate on near-expiry, or when periodic interval reached</text>

      {/* ── Sub-panel B: Per-Request Issuance ── */}
      <rect x={panelX} y={bPanelY} width={panelW} height={bPanelH} rx="16" className="sgn-panel" stroke="#a7f3d0" />
      <rect x="26" y={bPanelY + 14} width="206" height="24" rx="12" fill="#ecfdf5" stroke="#6ee7b7" strokeWidth="1" />
      <text x="129" y={bPanelY + 30} textAnchor="middle" className="sgn-chip-title" fill="#065f46">B · Per-Request Issuance</text>
      <text x="248" y={bPanelY + 30} className="sgn-chip-caption">on every RegisterRequest</text>

      <rect x="36" y={bPanelY + 54} width="144" height={nodeH} rx="14" fill="#eff6ff" stroke="#3b82f6" strokeWidth="1.6" filter="url(#sgn-shadow)" />
      <text x="108" y={bPanelY + 78} textAnchor="middle" className="sgn-node-title" fill="#1e40af">Actor Peer</text>
      <text x="108" y={bPanelY + 96} textAnchor="middle" className="sgn-node-subtitle" fill="#2563eb">requests registration</text>

      <rect x="258" y={bPanelY + 54} width="240" height={nodeH} rx="14" fill="#eef2ff" stroke="#6366f1" strokeWidth="1.6" filter="url(#sgn-shadow)" />
      <text x="378" y={bPanelY + 74} textAnchor="middle" className="sgn-node-title" fill="#3730a3">AIS</text>
      <text x="378" y={bPanelY + 90} textAnchor="middle" className="sgn-chip-caption" fill="#6366f1">verify MFR • build claims • select active key</text>
      <text x="378" y={bPanelY + 102} textAnchor="middle" className="sgn-chip-caption" fill="#6366f1">call Sign RPC • assemble credential bundle</text>

      <rect x="664" y={bPanelY + 54} width="144" height={nodeH} rx="14" fill="#fffbeb" stroke="#d97706" strokeWidth="1.6" filter="url(#sgn-shadow)" />
      <text x="736" y={bPanelY + 78} textAnchor="middle" className="sgn-node-title" fill="#92400e">Signer (KS)</text>
      <text x="736" y={bPanelY + 96} textAnchor="middle" className="sgn-node-subtitle" fill="#d97706">returns signature bytes</text>

      <line x1="180" y1={bPanelY + 80} x2="258" y2={bPanelY + 80} stroke="#3b82f6" strokeWidth="1.6" markerEnd="url(#sgn-ab)" />
      <rect x="163" y={bPanelY + 58} width="112" height="16" rx="8" className="sgn-label-chip" />
      <text x="219" y={bPanelY + 70} textAnchor="middle" className="sgn-arrow-label" fill="#1d4ed8">identity + attestation</text>

      <line x1="498" y1={bPanelY + 80} x2="664" y2={bPanelY + 80} stroke="#d97706" strokeWidth="1.6" markerEnd="url(#sgn-ao)" />
      <rect x="493" y={bPanelY + 58} width="176" height="16" rx="8" className="sgn-label-chip" />
      <text x="581" y={bPanelY + 70} textAnchor="middle" className="sgn-arrow-label" fill="#92400e">Sign RPC: key_id + claims_bytes</text>

      <line x1="664" y1={bPanelY + 96} x2="498" y2={bPanelY + 96} stroke="#10b981" strokeWidth="1.5" markerEnd="url(#sgn-ag)" />
      <rect x="537" y={bPanelY + 106} width="88" height="16" rx="8" className="sgn-label-chip" />
      <text x="581" y={bPanelY + 118} textAnchor="middle" className="sgn-arrow-label" fill="#059669">signature bytes</text>

      <line x1="258" y1={bPanelY + 96} x2="180" y2={bPanelY + 96} stroke="#10b981" strokeWidth="1.6" markerEnd="url(#sgn-ag)" />
      <rect x="154" y={bPanelY + 106} width="130" height="16" rx="8" className="sgn-label-chip" />
      <text x="219" y={bPanelY + 118} textAnchor="middle" className="sgn-arrow-label" fill="#059669">RegisterResponse bundle</text>

      <rect x="36" y={bPanelY + 130} width="828" height="34" rx="10" fill="#f8fafc" stroke="#dbe4f0" strokeWidth="1" />
      <text x="450" y={bPanelY + 144} textAnchor="middle" fontSize="9.5" fontWeight="600" fill="#166534">Response bundle</text>
      <text x="450" y={bPanelY + 157} textAnchor="middle" className="sgn-detail-text" fill="#16a34a">AIdCredential + TurnCredential + Verifier + KeyID</text>

      <text x="450" y={bPanelY + 188} textAnchor="middle" className="sgn-muted">AIS ↔ Signer transport: cluster-private gRPC with nonce-auth</text>
    </svg>
  );
}

function VerificationDiagram() {
  const totalH = 472;
  const panelX = 10;
  const panelW = 880;
  const nodeH = 62;

  const cPanelY = 10;
  const cPanelH = 218;
  const dPanelY = cPanelY + cPanelH + 16;
  const dPanelH = 218;

  return (
    <svg
      viewBox={`0 0 900 ${totalH}`}
      role="img"
      aria-label="Runtime Verification Flow"
      className="block h-auto w-full"
      preserveAspectRatio="xMidYMin meet"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <filter id="sgn-shadow" x="-20%" y="-20%" width="140%" height="140%">
          <feDropShadow dx="0" dy="4" stdDeviation="8" floodColor="#0f172a" floodOpacity="0.07" />
        </filter>
        <marker id="sgn-ag" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#10b981" />
        </marker>
        <marker id="sgn-ap" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#8b5cf6" />
        </marker>
        <marker id="sgn-at" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#f97316" />
        </marker>
        <marker id="sgn-ak" markerWidth="7" markerHeight="5" refX="7" refY="2.5" orient="auto">
          <path d="M0,0 L7,2.5 L0,5" fill="#22c55e" />
        </marker>
      </defs>
      <style>{`
        text { font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
        .sgn-muted { fill: #64748b; font-size: 11.5px; }
        .sgn-chip-title { font-size: 10.5px; font-weight: 700; }
        .sgn-chip-caption, .sgn-arrow-label { font-size: 9.4px; }
        .sgn-chip-caption { fill: #64748b; }
        .sgn-node-title { font-size: 12.5px; font-weight: 700; }
        .sgn-node-subtitle { font-size: 9px; }
        .sgn-panel { fill: #ffffff; stroke-width: 1.15px; }
        .sgn-label-chip { fill: #ffffff; stroke: #e2e8f0; stroke-width: 1; }
      `}</style>

      {/* ── Sub-panel C: Signaling path ── */}
      <rect x={panelX} y={cPanelY} width={panelW} height={cPanelH} rx="16" className="sgn-panel" stroke="#c4b5fd" />
      <rect x="26" y={cPanelY + 14} width="172" height="24" rx="12" fill="#ede9fe" stroke="#a78bfa" strokeWidth="1" />
      <text x="112" y={cPanelY + 28} textAnchor="middle" className="sgn-chip-title" fill="#5b21b6">C · Signaling Path</text>
      <text x="218" y={cPanelY + 28} className="sgn-chip-caption">offline Ed25519 verification</text>

      {(() => {
        const peerX = panelX + 24;
        const sigX = panelX + 252;
        const kcX = panelX + 552;
        const cNodeY = cPanelY + 72;
        return (
          <>
            <rect x={peerX} y={cNodeY} width="140" height={nodeH} rx="14" fill="#eff6ff" stroke="#3b82f6" strokeWidth="1.6" filter="url(#sgn-shadow)" />
            <text x={peerX + 70} y={cNodeY + 24} textAnchor="middle" className="sgn-node-title" fill="#1e40af">Actor Peer</text>
            <text x={peerX + 70} y={cNodeY + 42} textAnchor="middle" className="sgn-node-subtitle" fill="#2563eb">opens WebSocket session</text>

            <rect x={sigX} y={cNodeY} width="220" height={nodeH} rx="14" fill="#f5f3ff" stroke="#8b5cf6" strokeWidth="1.6" filter="url(#sgn-shadow)" />
            <text x={sigX + 110} y={cNodeY + 20} textAnchor="middle" className="sgn-node-title" fill="#5b21b6">Signaling</text>
            <text x={sigX + 110} y={cNodeY + 36} textAnchor="middle" className="sgn-chip-caption" fill="#7c3aed">Ed25519 check • decode claims</text>
            <text x={sigX + 110} y={cNodeY + 48} textAnchor="middle" className="sgn-chip-caption" fill="#7c3aed">check expires • realm • actor_id</text>

            <rect x={kcX} y={cNodeY} width="194" height={nodeH} rx="14" fill="#f0fdf4" stroke="#22c55e" strokeWidth="1.3" filter="url(#sgn-shadow)" />
            <text x={kcX + 97} y={cNodeY + 24} textAnchor="middle" fontSize="11.2" fontWeight="600" fill="#166534">signaling_key_cache.db</text>
            <text x={kcX + 97} y={cNodeY + 42} textAnchor="middle" className="sgn-node-subtitle" fill="#16a34a">lookup verifier by key_id</text>

            <line x1={peerX + 140} y1={cNodeY + 16} x2={sigX} y2={cNodeY + 16} stroke="#8b5cf6" strokeWidth="1.6" markerEnd="url(#sgn-ap)" />
            <rect x={154} y={cNodeY - 18} width="128" height="16" rx="8" className="sgn-label-chip" />
            <text x={218} y={cNodeY - 6} textAnchor="middle" className="sgn-arrow-label" fill="#7c3aed">upgrade request + credential</text>

            <line x1={sigX + 220} y1={cNodeY + 16} x2={kcX} y2={cNodeY + 16} stroke="#22c55e" strokeWidth="1.5" markerEnd="url(#sgn-ak)" />
            <rect x={467} y={cNodeY - 18} width="110" height="16" rx="8" className="sgn-label-chip" />
            <text x={522} y={cNodeY - 6} textAnchor="middle" className="sgn-arrow-label" fill="#16a34a">resolve verifier locally</text>

            <line x1={kcX} y1={cNodeY + 48} x2={sigX + 220} y2={cNodeY + 48} stroke="#22c55e" strokeWidth="1.4" strokeDasharray="5 4" markerEnd="url(#sgn-ak)" />
            <rect x={480} y={cNodeY + 66} width="84" height="16" rx="8" className="sgn-label-chip" />
            <text x={522} y={cNodeY + 78} textAnchor="middle" className="sgn-arrow-label" fill="#16a34a">verifier material</text>

            <line x1={sigX} y1={cNodeY + 48} x2={peerX + 140} y2={cNodeY + 48} stroke="#10b981" strokeWidth="1.4" strokeDasharray="5 4" markerEnd="url(#sgn-ag)" />
            <rect x={158} y={cNodeY + 66} width="120" height="16" rx="8" className="sgn-label-chip" />
            <text x={218} y={cNodeY + 78} textAnchor="middle" className="sgn-arrow-label" fill="#059669">session accepted or denied</text>
          </>
        );
      })()}
      <text x="450" y={cPanelY + cPanelH - 18} textAnchor="middle" className="sgn-muted">AIS propagates verifying keys to shared store; Signaling verifies locally without calling Signer</text>

      {/* ── Sub-panel D: TURN path ── */}
      <rect x={panelX} y={dPanelY} width={panelW} height={dPanelH} rx="16" className="sgn-panel" stroke="#fed7aa" />
      <rect x="26" y={dPanelY + 14} width="148" height="24" rx="12" fill="#ffedd5" stroke="#fdba74" strokeWidth="1" />
      <text x="100" y={dPanelY + 30} textAnchor="middle" className="sgn-chip-title" fill="#9a3412">D · TURN Path</text>
      <text x="192" y={dPanelY + 30} className="sgn-chip-caption">independent HMAC-SHA1 auth</text>

      {(() => {
        const peerX = panelX + 24;
        const turnX = panelX + 250;
        const stepsX = panelX + 514;
        const dNodeY = dPanelY + 74;
        return (
          <>
            <rect x={peerX} y={dNodeY} width="150" height={nodeH} rx="14" fill="#eff6ff" stroke="#3b82f6" strokeWidth="1.6" filter="url(#sgn-shadow)" />
            <text x={peerX + 75} y={dNodeY + 24} textAnchor="middle" className="sgn-node-title" fill="#1e40af">Actor Peer</text>
            <text x={peerX + 75} y={dNodeY + 42} textAnchor="middle" className="sgn-node-subtitle" fill="#2563eb">uses turn_credential</text>

            <rect x={turnX} y={dNodeY} width="160" height={nodeH} rx="14" fill="#fff7ed" stroke="#f97316" strokeWidth="1.6" filter="url(#sgn-shadow)" />
            <text x={turnX + 80} y={dNodeY + 24} textAnchor="middle" className="sgn-node-title" fill="#9a3412">TURN</text>
            <text x={turnX + 80} y={dNodeY + 42} textAnchor="middle" className="sgn-node-subtitle" fill="#ea580c">TURN auth handler</text>

            <rect x={stepsX} y={dNodeY - 2} width="282" height="72" rx="14" fill="#fefce8" stroke="#fde68a" strokeWidth="1.2" />
            <text x={stepsX + 141} y={dNodeY + 18} textAnchor="middle" fontSize="9.2" fontWeight="700" fill="#92400e">TURN auth_handle steps</text>
            <text x={stepsX + 141} y={dNodeY + 34} textAnchor="middle" className="sgn-arrow-label" fill="#b45309">split username → expires_at + actor_id</text>
            <text x={stepsX + 141} y={dNodeY + 48} textAnchor="middle" className="sgn-arrow-label" fill="#b45309">reject expired credential before auth</text>
            <text x={stepsX + 141} y={dNodeY + 62} textAnchor="middle" className="sgn-arrow-label" fill="#b45309">derive HMAC-SHA1 password → MD5 key</text>

            <line x1={peerX + 150} y1={dNodeY + 16} x2={turnX} y2={dNodeY + 16} stroke="#f97316" strokeWidth="1.6" markerEnd="url(#sgn-at)" />
            <rect x={159} y={dNodeY - 18} width="126" height="16" rx="8" className="sgn-label-chip" />
            <text x={222} y={dNodeY - 6} textAnchor="middle" className="sgn-arrow-label" fill="#ea580c">time-limited TURN credential</text>

            <line x1={turnX + 160} y1={dNodeY + 16} x2={stepsX} y2={dNodeY + 16} stroke="#f97316" strokeWidth="1.4" markerEnd="url(#sgn-at)" />
            <rect x={430} y={dNodeY - 18} width="84" height="16" rx="8" className="sgn-label-chip" />
            <text x={472} y={dNodeY - 6} textAnchor="middle" className="sgn-arrow-label" fill="#ea580c">run auth_handle</text>

            <line x1={stepsX} y1={dNodeY + 48} x2={turnX + 160} y2={dNodeY + 48} stroke="#10b981" strokeWidth="1.3" strokeDasharray="5 4" markerEnd="url(#sgn-ag)" />
            <rect x={406} y={dNodeY + 66} width="132" height="16" rx="8" className="sgn-label-chip" />
            <text x={472} y={dNodeY + 78} textAnchor="middle" className="sgn-arrow-label" fill="#059669">derived integrity key</text>

            <line x1={turnX} y1={dNodeY + 48} x2={peerX + 150} y2={dNodeY + 48} stroke="#10b981" strokeWidth="1.3" strokeDasharray="5 4" markerEnd="url(#sgn-ag)" />
            <rect x={161} y={dNodeY + 66} width="122" height="16" rx="8" className="sgn-label-chip" />
            <text x={222} y={dNodeY + 78} textAnchor="middle" className="sgn-arrow-label" fill="#059669">allow or reject allocation</text>
          </>
        );
      })()}
      <text x="450" y={dPanelY + dPanelH - 18} textAnchor="middle" className="sgn-muted">AIS issues TurnCredential; TURN validates independently without Ed25519 state</text>
    </svg>
  );
}

function KeyLifecycleBar({ config }: { config: Record<string, unknown> }) {
  const ttl = Number(config.key_ttl_seconds ?? 3600);
  const tolerance = Number(config.tolerance_seconds ?? 300);
  const totalSpan = ttl + tolerance;
  const activePercent = Math.max(8, (ttl / totalSpan) * 100);
  const tolerancePercent = Math.max(8, 100 - activePercent);

  return (
    <div className="w-full">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <p className="text-[11.5px] text-slate-500">
          Total validity = {ttl}s active + {tolerance}s tolerance
        </p>
        <span className="max-w-full rounded-full border border-slate-200 bg-slate-50 px-3 py-1 text-[10.5px] font-bold leading-4 text-slate-500 uppercase tracking-tight">
          cleanup deletes keys only after tolerance
        </span>
      </div>

      <div className="mt-4 overflow-hidden rounded-[14px] border border-slate-200 bg-slate-50">
        <div className="flex h-12 text-[12.5px] font-bold">
          <div
            className="flex items-center justify-center bg-emerald-100 text-emerald-800"
            style={{ width: `${activePercent}%` }}
          >
            Active ({ttl}s)
          </div>
          <div
            className="flex items-center justify-center bg-amber-100 text-amber-800"
            style={{ width: `${tolerancePercent}%` }}
          >
            Tolerance ({tolerance}s)
          </div>
        </div>
      </div>

      <div className="mt-4 grid gap-3 text-[11.5px] text-slate-600 md:grid-cols-3">
        <div className="rounded-[14px] border border-emerald-100 bg-emerald-50 p-4">
          <p className="font-bold text-emerald-700">Created → Expires</p>
          <p className="mt-1.5 leading-relaxed">Active signing window. Signer can sign new payloads, and verifiers accept the key as fresh.</p>
        </div>
        <div className="rounded-[14px] border border-amber-100 bg-amber-50 p-4">
          <p className="font-bold text-amber-700">Expires → Cleanup</p>
          <p className="mt-1.5 leading-relaxed">Grace period. New signing has moved to a fresher key, but verifiers still accept this key to support late-arriving credentials.</p>
        </div>
        <div className="rounded-[14px] border border-slate-200 bg-slate-50 p-4">
          <p className="font-bold text-slate-700">After cleanup</p>
          <p className="mt-1.5 leading-relaxed">The key is purged from verifier caches. Any remaining credentials using this key will now fail verification.</p>
        </div>
      </div>
    </div>
  );
}


export function SignerService() {
  const [data, setData] = useState<ServiceDetail | null>(null);
  const [keys, setKeys] = useState<KeyEntry[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [error, setError] = useState("");
  const [cleaning, setCleaning] = useState(false);
  const [cleanupMsg, setCleanupMsg] = useState("");
  const fallbackConfig: Record<string, unknown> = {
    key_ttl_seconds: 3600,
    tolerance_seconds: 86400,
  };

  function buildFallbackDetail(status: ServiceStatus | null): ServiceDetail {
    return {
      enabled: status != null,
      status,
      config: fallbackConfig,
    };
  }

  function applyFallbackState(status: ServiceStatus | null) {
    setData(buildFallbackDetail(status));
    setKeys([]);
    setTotalCount(0);
    setError("");
  }

  const fetchData = useCallback(async () => {
    const keysPromise = api.getSignerKeys().catch(() => ({ keys: [], total_count: 0 }));

    try {
      const [detail, k] = await Promise.all([
        api.getServiceDetail("signer"),
        keysPromise,
      ]);
      setData(detail);
      setKeys(k.keys);
      setTotalCount(k.total_count);
      setError("");
    } catch (err) {
      try {
        const [{ services }, k] = await Promise.all([
          api.getServices(),
          keysPromise,
        ]);
        const signerStatus = services.find((service) => service.name === "signer") ?? null;
        setData(buildFallbackDetail(signerStatus));
        setKeys(k.keys);
        setTotalCount(k.total_count);
        setError("");
      } catch {
        applyFallbackState(null);
      }
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  const diagramConfig = data?.config ?? fallbackConfig;

  if (!data && !error) {
    return <div className="text-sm text-gray-500">Loading...</div>;
  }

  return (
    <ServicePageLayout
      title="Signer Service"
      description="Signing Oracle — generates Ed25519 key pairs and signs on behalf of AIS; private keys never leave the process"
    >
      {error && !data && (
        <div className="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-700">
          {error}
        </div>
      )}

      {data && (
        <StatusSection
          enabled={data.enabled}
          healthy={data.status?.is_healthy}
          disabledHint={
            <>
              This service is not enabled. Set the Signer bit (bit 4) in the <code>enable</code> bitmask to activate it.
              The issuance and verification diagrams below still describe the runtime design.
            </>
          }
        />
      )}

      {data?.enabled && <ServiceMetrics status={data.status} storageKey="signer" />}

      <HowItWorks storageKey="signer_v3" defaultExpanded>
        {/* ── Concept cards ── */}
        <div className="grid gap-3 sm:grid-cols-3">
          <div className="rounded-xl border border-gray-100 bg-white px-4 py-4">
            <p className="text-sm font-semibold text-gray-700">1. Signing boundary</p>
            <p className="mt-1.5 text-xs text-gray-500">
              AIS asks Signer to generate keys and sign payloads. Private Ed25519 keys stay inside Signer's configured storage backend.
            </p>
          </div>
          <div className="rounded-xl border border-gray-100 bg-white px-4 py-4">
            <p className="text-sm font-semibold text-gray-700">2. Verifier distribution</p>
            <p className="mt-1.5 text-xs text-gray-500">
              AIS persists the verifier into local <code className="rounded bg-gray-100 px-1 py-0.5 text-[11px]">ais_keys.db</code> and shared <code className="rounded bg-gray-100 px-1 py-0.5 text-[11px]">signaling_key_cache.db</code> so Signaling verifies locally.
            </p>
          </div>
          <div className="rounded-xl border border-gray-100 bg-white px-4 py-4">
            <p className="text-sm font-semibold text-gray-700">3. TURN split</p>
            <p className="mt-1.5 text-xs text-gray-500">
              TURN does not verify Ed25519. It only checks the HMAC-SHA1 credential derived from{" "}
              <code className="rounded bg-gray-100 px-1 py-0.5 text-[11px]">turn_secret</code>.
            </p>
          </div>
        </div>

        {/* ── Section 1: Issuance ── */}
        <div className="mt-10">
          <h3 style={sectionTitleStyle}>1. Credential Issuance</h3>
          <p className="mt-1" style={sectionSubtitleStyle}>
            AIS refreshes verifier material in the background, then signs each register flow through cluster-private RPC.
          </p>
          <div className="mt-4 w-full rounded-[24px] border border-blue-100 bg-white p-4 sm:p-6 shadow-[0_8px_30px_rgba(15,23,42,0.04)]">
            <IssuanceDiagram />
          </div>
        </div>

        {/* ── Section 2: Verification ── */}
        <div className="mt-10">
          <h3 style={sectionTitleStyle}>2. Runtime Verification</h3>
          <p className="mt-1" style={sectionSubtitleStyle}>
            Signaling verifies Ed25519 locally with shared verifier store, while TURN follows its own HMAC-SHA1 path.
          </p>
          <div className="mt-4 w-full rounded-[24px] border border-purple-100 bg-white p-4 sm:p-6 shadow-[0_8px_30px_rgba(15,23,42,0.04)]">
            <VerificationDiagram />
          </div>
        </div>

        {/* ── Section 3: Bundle ── */}
        <div className="mt-10">
          <h3 style={sectionTitleStyle}>3. Credential Bundle</h3>
          <p className="mt-1" style={sectionSubtitleStyle}>
            AIS returns two credential families together: Ed25519-backed identity material and the separate TURN HMAC credential.
          </p>
          <div className="mt-4 w-full rounded-[24px] border border-violet-100 bg-white p-4 sm:p-6 shadow-[0_8px_30px_rgba(15,23,42,0.04)]">
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="rounded-[14px] border border-violet-100 bg-violet-50 p-5 text-[11.5px] text-slate-600">
                <div className="inline-flex items-center rounded-full border border-violet-200 bg-white px-3 py-1 text-[10.5px] font-bold text-violet-700 uppercase tracking-tight">
                  Ed25519 path
                </div>
                <p className="mt-4 text-[12.5px] font-bold text-violet-700">AIdCredential</p>
                <p className="mt-2 leading-relaxed">Claims + <code className="rounded bg-white px-1.5 py-0.5 text-[10.5px] font-bold">key_id</code> + Ed25519 signature</p>
              </div>
              <div className="rounded-[14px] border border-orange-100 bg-orange-50 p-5 text-[11.5px] text-slate-600">
                <div className="inline-flex items-center rounded-full border border-orange-200 bg-white px-3 py-1 text-[10.5px] font-bold text-orange-700 uppercase tracking-tight">
                  TURN path
                </div>
                <p className="mt-4 text-[12.5px] font-bold text-orange-700">TurnCredential</p>
                <p className="mt-2 leading-relaxed">Username = <code className="rounded bg-white px-1.5 py-0.5 text-[10.5px] font-bold">expires:actor_id</code></p>
                <p className="mt-1 leading-relaxed">Password = HMAC-SHA1(secret, username)</p>
              </div>
            </div>
          </div>
        </div>

        {/* ── Section 4: Lifecycle ── */}
        <div className="mt-10">
          <h3 style={sectionTitleStyle}>4. Signing Key Lifecycle</h3>
          <p className="mt-1" style={sectionSubtitleStyle}>
            Key validity keeps a clear active window followed by a grace period so older credentials can still verify before cleanup.
          </p>
          <div className="mt-4 w-full rounded-[24px] border border-indigo-100 bg-white p-4 sm:p-6 shadow-[0_8px_30px_rgba(15,23,42,0.04)]">
            <KeyLifecycleBar config={diagramConfig} />
          </div>
        </div>

        {/* ── Key concepts ── */}
        <div className="mt-8 rounded-2xl border border-gray-100 bg-gray-50/50 px-5 py-5">
          <p className="text-[11px] font-bold text-gray-400 uppercase tracking-widest">Key concepts</p>
          <ul className="mt-3 space-y-2 text-[12px] text-gray-500 list-disc pl-5">
            <li className="leading-relaxed">
              <strong className="text-gray-700 font-semibold">Signer boundary</strong> — private Ed25519 keys stay inside Signer's storage backend; AIS only receives the verifying key, expiry metadata, and signatures.
            </li>
            <li className="leading-relaxed">
              <strong className="text-gray-700 font-semibold">Local verification</strong> — AIS persists verifying keys into shared KeyCache; Signaling verifies AIdCredentials locally by <code className="rounded bg-gray-100 px-1 py-0.5 text-[11px]">key_id</code> without a Signer round-trip.
            </li>
            <li className="leading-relaxed">
              <strong className="text-gray-700 font-semibold">TURN is separate</strong> — TURN does not use KeyCache or Ed25519; it independently validates the HMAC-SHA1 credential derived from <code className="rounded bg-gray-100 px-1 py-0.5 text-[11px]">turn_secret</code>.
            </li>
            <li className="leading-relaxed">
              <strong className="text-gray-700 font-semibold">Lifecycle window</strong> — <code className="rounded bg-gray-100 px-1 py-0.5 text-[11px]">key_ttl_seconds</code> defines the active window; <code className="rounded bg-gray-100 px-1 py-0.5 text-[11px]">tolerance_seconds</code> keeps old credentials verifiable until cleanup.
            </li>
          </ul>
        </div>
      </HowItWorks>

      {data?.config_fields && (
        <ConfigSection storageKey="signer" fields={data.config_fields} onRefresh={fetchData} />
      )}

      {data?.enabled && (
        <CollapsibleCard storageKey="signer_keys" title="Keys">
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
                  const r = await api.cleanupSignerKeys();
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
      )}
    </ServicePageLayout>
  );
}
