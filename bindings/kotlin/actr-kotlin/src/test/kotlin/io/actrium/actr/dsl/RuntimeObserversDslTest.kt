package io.actrium.actr.dsl

import io.actrium.actr.CredentialObserverBridge
import io.actrium.actr.MailboxObserverBridge
import io.actrium.actr.SignalingObserverBridge
import io.actrium.actr.WebRtcObserverBridge
import io.actrium.actr.WebSocketObserverBridge
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull

class RuntimeObserversDslTest {
    @Test
    fun `RuntimeObservers typealias aliases the generated class`() {
        assertEquals(io.actrium.actr.RuntimeObservers::class, RuntimeObservers::class)
    }

    @Test
    fun `runtimeObservers factory exposes all observer slots`() {
        // Compile-time signature check. Not invoked — building a RuntimeObservers
        // calls into native code, which JVM unit tests cannot exercise.
        val factory: (
            SignalingObserverBridge?,
            WebSocketObserverBridge?,
            WebRtcObserverBridge?,
            CredentialObserverBridge?,
            MailboxObserverBridge?,
        ) -> RuntimeObservers = ::runtimeObservers
        assertNotNull(factory)
    }
}
