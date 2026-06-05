package io.actor_rtc.actr.dsl

import io.actor_rtc.actr.ActrType
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class ActrTypeTest {

    @Test
    fun `toActrType parses canonical colon format`() {
        val type = "acme:EchoService:1.0.0".toActrType()
        assertEquals("acme", type.manufacturer)
        assertEquals("EchoService", type.name)
        assertEquals("1.0.0", type.version)
    }

    @Test
    fun `toActrType parses with complex manufacturer name`() {
        val type = "demo2:EchoService:2.0.1".toActrType()
        assertEquals("demo2", type.manufacturer)
        assertEquals("EchoService", type.name)
        assertEquals("2.0.1", type.version)
    }

    @Test
    fun `toActrType parses with semver version`() {
        val type = "actrium:DataStreamConcurrentServer:0.1.0-beta".toActrType()
        assertEquals("actrium", type.manufacturer)
        assertEquals("DataStreamConcurrentServer", type.name)
        assertEquals("0.1.0-beta", type.version)
    }

    @Test
    fun `toActrType rejects string without colon separator`() {
        assertFailsWith<IllegalArgumentException> {
            "acmeEchoService1.0.0".toActrType()
        }
    }

    @Test
    fun `toActrType rejects string with only one colon`() {
        assertFailsWith<IllegalArgumentException> {
            "acme:EchoService".toActrType()
        }
    }

    @Test
    fun `toActrType rejects empty version`() {
        assertFailsWith<IllegalArgumentException> {
            "acme:EchoService:".toActrType()
        }
    }

    @Test
    fun `toActrType rejects blank version`() {
        assertFailsWith<IllegalArgumentException> {
            "acme:EchoService:   ".toActrType()
        }
    }

    @Test
    fun `toActrType handles version with dots and dashes`() {
        val type = "acme:MyService:1.2.3-beta.4".toActrType()
        assertEquals("1.2.3-beta.4", type.version)
    }

    // --- actrType() factory function ---

    @Test
    fun `actrType factory creates ActrType from positional args`() {
        val type = actrType("demo2", "EchoService", "2.0.0")
        assertEquals("demo2", type.manufacturer)
        assertEquals("EchoService", type.name)
        assertEquals("2.0.0", type.version)
    }

    // --- actrType builder DSL ---

    @Test
    fun `actrType builder creates ActrType from DSL block`() {
        val type = actrType {
            manufacturer = "acme"
            name = "EchoService"
            version = "1.0.0"
        }
        assertEquals("acme", type.manufacturer)
        assertEquals("EchoService", type.name)
        assertEquals("1.0.0", type.version)
    }

    @Test
    fun `actrType builder rejects blank manufacturer`() {
        assertFailsWith<IllegalArgumentException> {
            actrType {
                manufacturer = ""
                name = "EchoService"
                version = "1.0.0"
            }
        }
    }

    @Test
    fun `actrType builder rejects blank name`() {
        assertFailsWith<IllegalArgumentException> {
            actrType {
                manufacturer = "acme"
                name = ""
                version = "1.0.0"
            }
        }
    }

    @Test
    fun `actrType builder rejects blank version`() {
        assertFailsWith<IllegalArgumentException> {
            actrType {
                manufacturer = "acme"
                name = "EchoService"
                version = ""
            }
        }
    }

    // --- Round-trip ---

    @Test
    fun `ActrType fields match canonical string representation`() {
        val type = actrType("demo2", "EchoService", "1.0.0")
        val canonical = "${type.manufacturer}:${type.name}:${type.version}"
        assertEquals("demo2:EchoService:1.0.0", canonical)

        val parsed = canonical.toActrType()
        assertEquals(type.manufacturer, parsed.manufacturer)
        assertEquals(type.name, parsed.name)
        assertEquals(type.version, parsed.version)
    }
}