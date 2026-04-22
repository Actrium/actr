// SPDX-License-Identifier: Apache-2.0
//
// C implementation of an actr workload. Demonstrates that the
// actr:workload contract (core/framework/wit/actr-workload.wit) can be
// authored in C via wit-bindgen c + clang (wasm32-wasip2) + wasm-component-ld.
//
// Semantics: dispatch echoes the incoming payload back with an "echo: "
// prefix. All observation hooks are implemented as infallible no-ops.
//
// Memory ownership note (canonical ABI):
//   - Inbound record parameters (rpc-envelope, peer-event, error-event)
//     are owned by the guest: we must call the generated `*_free` helpers
//     before returning, otherwise the host leaks the guest-side allocation.
//   - The `list<u8>` result from dispatch is transferred to the host, which
//     invokes cabi_realloc-backed free after reading it. We allocate with
//     malloc so the wit-bindgen runtime (which defers to libc free via
//     cabi_realloc) can release it.

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "actr_workload_guest.h"

// ─────────────────────────────────────────────────────────────────────────
// Inbound RPC dispatch
// ─────────────────────────────────────────────────────────────────────────

static const char kEchoPrefix[] = "echo: ";

bool exports_actr_workload_workload_dispatch(
    exports_actr_workload_workload_rpc_envelope_t *envelope,
    actr_workload_guest_list_u8_t *ret,
    exports_actr_workload_workload_actr_error_t *err) {
    (void)err;

    const size_t prefix_len = sizeof(kEchoPrefix) - 1;
    const size_t payload_len = envelope->payload.len;
    const size_t total_len = prefix_len + payload_len;

    // malloc(0) behaviour is implementation-defined; request at least 1 byte
    // so the returned pointer is always valid for the host.
    uint8_t *out = (uint8_t *)malloc(total_len > 0 ? total_len : 1);
    memcpy(out, kEchoPrefix, prefix_len);
    if (payload_len > 0) {
        memcpy(out + prefix_len, envelope->payload.ptr, payload_len);
    }

    ret->ptr = out;
    ret->len = total_len;

    // Release ownership of the inbound envelope (canonical ABI: guest owns
    // record parameters on entry).
    exports_actr_workload_workload_rpc_envelope_free(envelope);
    return true;
}

// ─────────────────────────────────────────────────────────────────────────
// Lifecycle hooks (fallible by WIT signature; we never surface errors)
// ─────────────────────────────────────────────────────────────────────────

bool exports_actr_workload_workload_on_start(
    exports_actr_workload_workload_actr_error_t *err) {
    (void)err;
    return true;
}

bool exports_actr_workload_workload_on_ready(
    exports_actr_workload_workload_actr_error_t *err) {
    (void)err;
    return true;
}

bool exports_actr_workload_workload_on_stop(
    exports_actr_workload_workload_actr_error_t *err) {
    (void)err;
    return true;
}

bool exports_actr_workload_workload_on_error(
    exports_actr_workload_workload_error_event_t *event,
    exports_actr_workload_workload_actr_error_t *err) {
    (void)err;
    exports_actr_workload_workload_error_event_free(event);
    return true;
}

// ─────────────────────────────────────────────────────────────────────────
// Signaling hooks (infallible)
// ─────────────────────────────────────────────────────────────────────────

void exports_actr_workload_workload_on_signaling_connecting(void) {}
void exports_actr_workload_workload_on_signaling_connected(void) {}
void exports_actr_workload_workload_on_signaling_disconnected(void) {}

// ─────────────────────────────────────────────────────────────────────────
// Transport hooks — WebSocket (infallible)
// ─────────────────────────────────────────────────────────────────────────

void exports_actr_workload_workload_on_websocket_connecting(
    exports_actr_workload_workload_peer_event_t *event) {
    exports_actr_workload_workload_peer_event_free(event);
}
void exports_actr_workload_workload_on_websocket_connected(
    exports_actr_workload_workload_peer_event_t *event) {
    exports_actr_workload_workload_peer_event_free(event);
}
void exports_actr_workload_workload_on_websocket_disconnected(
    exports_actr_workload_workload_peer_event_t *event) {
    exports_actr_workload_workload_peer_event_free(event);
}

// ─────────────────────────────────────────────────────────────────────────
// Transport hooks — WebRTC (infallible)
// ─────────────────────────────────────────────────────────────────────────

void exports_actr_workload_workload_on_webrtc_connecting(
    exports_actr_workload_workload_peer_event_t *event) {
    exports_actr_workload_workload_peer_event_free(event);
}
void exports_actr_workload_workload_on_webrtc_connected(
    exports_actr_workload_workload_peer_event_t *event) {
    exports_actr_workload_workload_peer_event_free(event);
}
void exports_actr_workload_workload_on_webrtc_disconnected(
    exports_actr_workload_workload_peer_event_t *event) {
    exports_actr_workload_workload_peer_event_free(event);
}

// ─────────────────────────────────────────────────────────────────────────
// Credential hooks (infallible; credential-event has no owned fields)
// ─────────────────────────────────────────────────────────────────────────

void exports_actr_workload_workload_on_credential_renewed(
    exports_actr_workload_workload_credential_event_t *event) {
    (void)event;
}
void exports_actr_workload_workload_on_credential_expiring(
    exports_actr_workload_workload_credential_event_t *event) {
    (void)event;
}

// ─────────────────────────────────────────────────────────────────────────
// Mailbox hook (infallible; backpressure-event is plain scalars)
// ─────────────────────────────────────────────────────────────────────────

void exports_actr_workload_workload_on_mailbox_backpressure(
    exports_actr_workload_workload_backpressure_event_t *event) {
    (void)event;
}
