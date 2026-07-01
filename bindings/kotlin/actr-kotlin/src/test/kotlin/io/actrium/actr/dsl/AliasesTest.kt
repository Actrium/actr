package io.actrium.actr.dsl

import io.actrium.actr.ActrId
import io.actrium.actr.ActrType
import io.actrium.actr.Realm
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class AliasesTest {
    private val peer = ActrId(Realm(7u), 1uL, ActrType("acme", "Echo", "1.0.0"))

    @Test
    fun `PeerEvent aliases the generated PeerEventBridge`() {
        assertEquals(io.actrium.actr.PeerEventBridge::class, PeerEvent::class)
    }

    @Test
    fun `WebRtcPeerStatus aliases the generated WebRtcPeerStatusBridge`() {
        assertEquals(io.actrium.actr.WebRtcPeerStatusBridge::class, WebRtcPeerStatus::class)
    }

    @Test
    fun `event and error aliases expose generated records`() {
        assertEquals(io.actrium.actr.ErrorEventBridge::class, ErrorEvent::class)
        assertEquals(io.actrium.actr.ErrorCategoryBridge::class, ErrorCategory::class)
        assertEquals(io.actrium.actr.CredentialEventBridge::class, CredentialEvent::class)
        assertEquals(io.actrium.actr.BackpressureEventBridge::class, BackpressureEvent::class)
    }

    @Test
    fun `PeerEvent carries every WebRtcPeerStatus value`() {
        assertEquals(
            WebRtcPeerStatus.CONNECTING,
            PeerEvent(peer, relayed = true, status = WebRtcPeerStatus.CONNECTING).status,
        )
        assertEquals(
            WebRtcPeerStatus.CONNECTED,
            PeerEvent(peer, relayed = false, status = WebRtcPeerStatus.CONNECTED).status,
        )
        assertEquals(
            WebRtcPeerStatus.RECOVERING,
            PeerEvent(peer, relayed = null, status = WebRtcPeerStatus.RECOVERING).status,
        )
        assertEquals(
            WebRtcPeerStatus.IDLE,
            PeerEvent(peer, relayed = null, status = WebRtcPeerStatus.IDLE).status,
        )
    }

    @Test
    fun `PeerEvent allows null status for WebSocket peers`() {
        val websocket = PeerEvent(peer, relayed = null, status = null)
        assertNull(websocket.status)
    }
}
