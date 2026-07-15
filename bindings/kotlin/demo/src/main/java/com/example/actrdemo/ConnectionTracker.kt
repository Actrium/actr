package com.example.actrdemo

import io.actrium.actr.ActrException
import io.actrium.actr.ActrId
import io.actrium.actr.dsl.ActrContext
import io.actrium.actr.dsl.PeerEvent
import io.actrium.actr.dsl.SignalingObserver
import io.actrium.actr.dsl.WebRtcObserver
import io.actrium.actr.dsl.WebRtcPeerStatus
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/**
 * Tracks ACTR transport readiness from host-side hooks and implements the
 * send-retry flow described in the WebRTC Hook usage doc.
 *
 * Two independent signals are kept separate, as the doc requires:
 *  - [SignalingObserver] drives [signalingReady] (service connection up/down).
 *  - [WebRtcObserver] drives per-peer [webRtcStatus]. ConnectionNotReady never
 *    mutates this map — it is a *send result*, not a connection-state hook.
 *
 * Send retry: when a send throws [ActrException.ConnectionNotReady], stash the
 * retry via [scheduleRetry]; [onConnected] (or a fallback timer armed with
 * `retryAfterMs`) re-sends once via [takeRetry].
 */
class ConnectionTracker {
    val signalingReady = AtomicBoolean(false)

    /** Per-target WebRTC readiness. An absent entry means Idle (lazy, not yet created). */
    val webRtcStatus = ConcurrentHashMap<ActrId, WebRtcPeerStatus>()

    /**
     * Single pending retry slot. A real multi-target app keys this by target;
     * this demo talks to one echo server, so one slot suffices.
     */
    private val pendingRetry = AtomicReference<(suspend () -> Unit)?>()
    private val retryTimer = AtomicReference<Job?>(null)

    /** Invoked on the runtime thread after every transition; hop to Main to refresh UI. */
    var onEvent: ((String) -> Unit)? = null

    /** Recommended send gate: signaling up, no in-flight retry, peer Idle/Connected. */
    fun canSend(): Boolean {
        if (!signalingReady.get()) return false
        if (pendingRetry.get() != null) return false
        val statuses = webRtcStatus.values
        if (statuses.isEmpty()) return true
        return statuses.all { it == WebRtcPeerStatus.IDLE || it == WebRtcPeerStatus.CONNECTED }
    }

    fun reset() {
        signalingReady.set(false)
        webRtcStatus.clear()
        retryTimer.getAndSet(null)?.cancel()
        pendingRetry.set(null)
    }

    val signalingObserver = object : SignalingObserver {
        override suspend fun onConnecting(ctx: ActrContext?) {
            signalingReady.set(false)
            emit("signaling: connecting")
        }

        override suspend fun onConnected(ctx: ActrContext?) {
            signalingReady.set(true)
            emit("signaling: connected")
        }

        override suspend fun onDisconnected(ctx: ActrContext) {
            signalingReady.set(false)
            emit("signaling: disconnected")
        }
    }

    val webRtcObserver = object : WebRtcObserver {
        override suspend fun onConnecting(ctx: ActrContext, e: PeerEvent) {
            webRtcStatus[e.peer] = WebRtcPeerStatus.CONNECTING
            emit("webrtc ${e.peer.serialNumber}: connecting")
        }

        override suspend fun onConnected(ctx: ActrContext, e: PeerEvent) {
            webRtcStatus[e.peer] = WebRtcPeerStatus.CONNECTED
            emit("webrtc ${e.peer.serialNumber}: connected (relayed=${e.relayed})")
            takeRetry()?.invoke()
        }

        override suspend fun onDisconnected(ctx: ActrContext, e: PeerEvent) {
            val s = e.status ?: WebRtcPeerStatus.IDLE
            webRtcStatus[e.peer] = s
            emit("webrtc ${e.peer.serialNumber}: ${s.name.lowercase()}")
        }
    }

    /**
     * Stash [retry] for later re-send. If [retryAfterMs] is present, arm a
     * fallback timer on [scope] so the message still goes out if `onConnected`
     * never arrives. The doc notes retryAfterMs is a hint, not a readiness promise.
     */
    fun scheduleRetry(
        retry: suspend () -> Unit,
        retryAfterMs: ULong?,
        scope: CoroutineScope,
    ) {
        pendingRetry.set(retry)
        retryAfterMs?.let { ms ->
            retryTimer.set(
                scope.launch {
                    delay(ms.toLong())
                    takeRetry()?.invoke()
                },
            )
        }
    }

    /** Atomically claim and clear the pending retry, cancelling any fallback timer. */
    fun takeRetry(): (suspend () -> Unit)? {
        retryTimer.getAndSet(null)?.cancel()
        return pendingRetry.getAndSet(null)
    }

    private fun emit(message: String) {
        onEvent?.invoke(message)
    }
}
