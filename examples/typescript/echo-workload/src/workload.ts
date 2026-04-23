// SPDX-License-Identifier: Apache-2.0
//
// Echo workload (TypeScript) — actr Component Model guest.
//
// Implements the `actr-workload-guest` world declared in
// `core/framework/wit/actr-workload.wit`. Built for `wasm32-wasip2` via
// `jco componentize` (StarlingMonkey JS engine + ComponentizeJS), the
// resulting binary is a Component that the actr host loads through
// `Component::from_binary`.
//
// NOTE (experimental): ComponentizeJS embeds the StarlingMonkey
// SpiderMonkey build and adds roughly 10 MB to the output Component.
// Cold-start and dispatch latency are materially higher than Rust /
// TinyGo / C guests. This example is a demo / compatibility probe,
// not a production path. It is intentionally excluded from CI.
//
// Minimal hook coverage:
//
//   - dispatch:  echo back the inbound payload prefixed with "echo: "
//   - onStart:   log a startup line via the host log sink
//
// The remaining 14 observation hooks are exported as no-ops so that
// ComponentizeJS can satisfy the full export surface required by the
// `workload` interface (absent exports would fail the Component link).
//
// Export shape: because `workload` is an *interface* (not a free-function)
// inside the world, ComponentizeJS expects a single named export
// `workload` that is an object carrying one camelCase method per WIT
// function. Host imports are reached through the
// "actr:workload/host@0.1.0" module specifier, resolved at componentize
// time against the world's import list.

// @ts-expect-error - resolved by ComponentizeJS against the world's host import
import { logMessage } from 'actr:workload/host@0.1.0';

// Mirror of the TS shape ComponentizeJS emits for the WIT types — see
// `core/framework/wit/actr-workload.wit` interface `types`.
interface RpcEnvelope {
    requestId: string;
    routeKey: string;
    payload: Uint8Array;
}

interface PeerEvent {
    peer: unknown;
    relayed?: boolean;
}

interface Timestamp {
    seconds: bigint;
    nanoseconds: number;
}

interface ErrorEvent {
    source: unknown;
    category: unknown;
    context: string;
    timestamp: Timestamp;
}

interface CredentialEvent {
    newExpiry: Timestamp;
}

interface BackpressureEvent {
    queueLen: bigint;
    threshold: bigint;
}

// ── Exports for `actr:workload/workload@0.1.0` ────────────────────────────
//
// Fallible hooks (`result<_, actr-error>`) use ComponentizeJS's
// "throw-on-error" convention: return normally for Ok, throw for Err.

export const workload = {
    // ── Inbound RPC dispatch ─────────────────────────────────────────────

    dispatch(envelope: RpcEnvelope): Uint8Array {
        const body = new TextDecoder().decode(envelope.payload);
        return new TextEncoder().encode('echo: ' + body);
    },

    // ── Lifecycle (4, fallible) ──────────────────────────────────────────

    onStart(): void {
        logMessage('info', 'TS echo workload started');
    },

    onReady(): void {
        // no-op
    },

    onStop(): void {
        // no-op
    },

    onError(_event: ErrorEvent): void {
        // no-op
    },

    // ── Signaling (3, infallible) ────────────────────────────────────────

    onSignalingConnecting(): void {},
    onSignalingConnected(): void {},
    onSignalingDisconnected(): void {},

    // ── Transport: WebSocket (3, infallible) ─────────────────────────────

    onWebsocketConnecting(_event: PeerEvent): void {},
    onWebsocketConnected(_event: PeerEvent): void {},
    onWebsocketDisconnected(_event: PeerEvent): void {},

    // ── Transport: WebRTC P2P (3, infallible) ────────────────────────────

    onWebrtcConnecting(_event: PeerEvent): void {},
    onWebrtcConnected(_event: PeerEvent): void {},
    onWebrtcDisconnected(_event: PeerEvent): void {},

    // ── Credential (2, infallible) ───────────────────────────────────────

    onCredentialRenewed(_event: CredentialEvent): void {},
    onCredentialExpiring(_event: CredentialEvent): void {},

    // ── Mailbox (1, infallible) ──────────────────────────────────────────

    onMailboxBackpressure(_event: BackpressureEvent): void {},
};
