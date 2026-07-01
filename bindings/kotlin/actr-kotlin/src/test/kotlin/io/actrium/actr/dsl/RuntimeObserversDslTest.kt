package io.actrium.actr.dsl

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
            SignalingObserver?,
            WebSocketObserver?,
            WebRtcObserver?,
            CredentialObserver?,
            MailboxObserver?,
        ) -> RuntimeObservers = ::runtimeObservers
        assertNotNull(factory)
    }
}
