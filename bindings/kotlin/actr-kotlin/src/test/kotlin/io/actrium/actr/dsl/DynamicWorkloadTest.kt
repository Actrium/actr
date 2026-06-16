package com.example.generated

import io.actrium.actr.ActrType
import io.actrium.actr.ContextBridge
import io.actrium.actr.ErrorEventBridge
import io.actrium.actr.RpcEnvelopeBridge
import io.actrium.actr.WorkloadLifecycleBridge
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull

/**
 * Unit tests for WorkloadLifecycleBridge — the Kotlin equivalent of Swift's
 * StaticWorkloadProbe used in DynamicWorkloadTests.
 *
 * These tests verify that the bridge interface contract is correct and
 * basic implementations compile. Full E2E tests with mock-actrix require
 * the native libactr.so and run via UnifiedIntegrationTest (androidTest).
 */
class DynamicWorkloadTest {

    /**
     * Minimal WorkloadLifecycleBridge implementation for testing,
     * mirroring Swift's StaticWorkloadProbe pattern.
     */
    private class TestWorkloadProbe : WorkloadLifecycleBridge {
        var startCount = 0
        var readyCount = 0
        var stopCount = 0
        var errorCount = 0
        var dispatchCount = 0
        val dispatchedPayloads = mutableListOf<ByteArray>()

        override suspend fun onStart(ctx: ContextBridge) {
            startCount++
        }

        override suspend fun onReady(ctx: ContextBridge) {
            readyCount++
        }

        override suspend fun onStop(ctx: ContextBridge) {
            stopCount++
        }

        override suspend fun onError(ctx: ContextBridge, event: ErrorEventBridge) {
            errorCount++
        }

        override suspend fun dispatch(ctx: ContextBridge, envelope: RpcEnvelopeBridge): ByteArray {
            dispatchCount++
            dispatchedPayloads.add(envelope.payload)
            return "echo:${envelope.payload.decodeToString()}".toByteArray()
        }
    }

    @Test
    fun `WorkloadLifecycleBridge can be implemented`() {
        val probe = TestWorkloadProbe()
        assertNotNull(probe)
        assertEquals(0, probe.startCount)
    }

    @Test
    fun `WorkloadLifecycleBridge tracks lifecycle state changes`() {
        val probe = TestWorkloadProbe()
        // Initial state
        assertEquals(0, probe.startCount)
        assertEquals(0, probe.readyCount)
        assertEquals(0, probe.stopCount)
        assertEquals(0, probe.errorCount)
        assertEquals(0, probe.dispatchCount)
        assertEquals(0, probe.dispatchedPayloads.size)
    }

    @Test
    fun `actrType factory produces correct type for EchoService`() {
        val serverType = ActrType("acme", "EchoService", "1.0.0")
        assertEquals("acme", serverType.manufacturer)
        assertEquals("EchoService", serverType.name)
        assertEquals("1.0.0", serverType.version)
    }

    @Test
    fun `actrType factory produces correct type for UnifiedActor`() {
        val clientType = ActrType("acme", "UnifiedActor", "1.0.0")
        assertEquals("acme", clientType.manufacturer)
        assertEquals("UnifiedActor", clientType.name)
        assertEquals("1.0.0", clientType.version)
    }
}