package io.actrium.actr.dsl

import io.actrium.actr.ActrException
import io.actrium.actr.ActrType
import java.io.File
import java.nio.file.Path
import kotlin.io.path.pathString
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class ManifestTest {
    private class FakeManifestResolver(
        var aliases: List<String> = listOf("EchoService", "DataStreamServer"),
    ) : ManifestResolver {
        var packageTypePath: String? = null
        var dependencyRequest: Pair<String, String>? = null
        var aliasListPath: String? = null
        var packageTypeError: ActrException? = null
        var aliasListError: ActrException? = null
        val dependencyErrors = mutableMapOf<String, ActrException>()

        override fun packageType(manifestPath: String): ActrType {
            packageTypePath = manifestPath
            packageTypeError?.let { throw it }
            return ActrType("acme", "test-actor", "1.0.0")
        }

        override fun resolveDependency(
            manifestPath: String,
            dependencyAlias: String,
        ): ActrType {
            dependencyRequest = manifestPath to dependencyAlias
            dependencyErrors[dependencyAlias]?.let { throw it }
            return when (dependencyAlias) {
                "EchoService" -> ActrType("acme", "EchoService", "1.0.0")
                "DataStreamServer" -> ActrType("acme", "DataStreamServer", "2.0.1")
                else -> throw ActrException.Config("Dependency '$dependencyAlias' not found")
            }
        }

        override fun dependencyAliases(manifestPath: String): List<String> {
            aliasListPath = manifestPath
            aliasListError?.let { throw it }
            return aliases
        }
    }

    @Test
    fun `Manifest from Path delegates using path string`() {
        val path = Path.of("/tmp/test-actr.toml")
        val resolver = FakeManifestResolver()
        val manifest = Manifest.from(path, resolver)

        val pkgType = manifest.packageType()

        assertEquals(path.pathString, resolver.packageTypePath)
        assertEquals("acme", pkgType.manufacturer)
        assertEquals("test-actor", pkgType.name)
        assertEquals("1.0.0", pkgType.version)
    }

    @Test
    fun `Manifest from File delegates using absolute path`() {
        val file = File("/tmp/test-actr.toml")
        val resolver = FakeManifestResolver()
        val manifest = Manifest.from(file, resolver)

        val aliases = manifest.dependencyAliases()

        assertEquals(file.absolutePath, resolver.aliasListPath)
        assertEquals(listOf("EchoService", "DataStreamServer"), aliases)
    }

    @Test
    fun `Manifest direct constructor accepts raw path string`() {
        val manifest = Manifest("/tmp/test-actr.toml")
        assertNotNull(manifest)
    }

    @Test
    fun `packageType resolves ActrType from manifest`() {
        val resolver = FakeManifestResolver()
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val pkgType = manifest.packageType()

        assertEquals("acme", pkgType.manufacturer)
        assertEquals("test-actor", pkgType.name)
        assertEquals("1.0.0", pkgType.version)
    }

    @Test
    fun `resolveManifestPackageActrType top-level with Path overload`() {
        val path = Path.of("/tmp/actr.toml")
        val resolver = FakeManifestResolver()

        val pkgType = resolveManifestPackageActrType(path, resolver)

        assertEquals(path.pathString, resolver.packageTypePath)
        assertEquals("acme", pkgType.manufacturer)
        assertEquals("test-actor", pkgType.name)
        assertEquals("1.0.0", pkgType.version)
    }

    @Test
    fun `resolveManifestPackageActrType top-level with File overload`() {
        val file = File("/tmp/actr.toml")
        val resolver = FakeManifestResolver()

        val pkgType = resolveManifestPackageActrType(file, resolver)

        assertEquals(file.absolutePath, resolver.packageTypePath)
        assertEquals("acme", pkgType.manufacturer)
        assertEquals("test-actor", pkgType.name)
    }

    @Test
    fun `packageType propagates config errors`() {
        val resolver =
            FakeManifestResolver().apply {
                packageTypeError = ActrException.Config("Failed to parse manifest")
            }
        val manifest = Manifest.from(Path.of("/nonexistent/path/actr.toml"), resolver)

        val err =
            assertFailsWith<ActrException.Config> {
                manifest.packageType()
            }

        assertTrue("parse" in err.message.orEmpty())
    }

    @Test
    fun `resolveDependency returns correct ActrType for known alias`() {
        val resolver = FakeManifestResolver()
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val depType = manifest.resolveDependency("EchoService")

        assertEquals("acme", depType.manufacturer)
        assertEquals("EchoService", depType.name)
        assertEquals("1.0.0", depType.version)
    }

    @Test
    fun `resolveDependency returns correct ActrType for second alias`() {
        val resolver = FakeManifestResolver()
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val depType = manifest.resolveDependency("DataStreamServer")

        assertEquals("acme", depType.manufacturer)
        assertEquals("DataStreamServer", depType.name)
        assertEquals("2.0.1", depType.version)
    }

    @Test
    fun `resolveManifestDependency top-level with Path overload`() {
        val path = Path.of("/tmp/actr.toml")
        val resolver = FakeManifestResolver()

        val depType = resolveManifestDependency(path, "EchoService", resolver)

        assertEquals(path.pathString to "EchoService", resolver.dependencyRequest)
        assertEquals("acme", depType.manufacturer)
        assertEquals("EchoService", depType.name)
        assertEquals("1.0.0", depType.version)
    }

    @Test
    fun `resolveManifestDependency top-level with File overload`() {
        val file = File("/tmp/actr.toml")
        val resolver = FakeManifestResolver()

        val depType = resolveManifestDependency(file, "EchoService", resolver)

        assertEquals(file.absolutePath to "EchoService", resolver.dependencyRequest)
        assertEquals("acme", depType.manufacturer)
        assertEquals("EchoService", depType.name)
    }

    @Test
    fun `resolveDependency propagates missing alias error`() {
        val resolver = FakeManifestResolver()
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val err =
            assertFailsWith<ActrException.Config> {
                manifest.resolveDependency("NonExistentAlias")
            }

        assertTrue("NonExistentAlias" in err.message.orEmpty())
    }

    @Test
    fun `dependencyAliases returns all declared aliases`() {
        val resolver = FakeManifestResolver()
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val aliases = manifest.dependencyAliases()

        assertEquals(2, aliases.size)
        assertTrue("EchoService" in aliases)
        assertTrue("DataStreamServer" in aliases)
    }

    @Test
    fun `resolveManifestDependencyAliasList top-level with Path overload`() {
        val path = Path.of("/tmp/actr.toml")
        val resolver = FakeManifestResolver()

        val aliases = resolveManifestDependencyAliasList(path, resolver)

        assertEquals(path.pathString, resolver.aliasListPath)
        assertEquals(2, aliases.size)
        assertTrue("EchoService" in aliases)
    }

    @Test
    fun `resolveManifestDependencyAliasList top-level with File overload`() {
        val file = File("/tmp/actr.toml")
        val resolver = FakeManifestResolver()

        val aliases = resolveManifestDependencyAliasList(file, resolver)

        assertEquals(file.absolutePath, resolver.aliasListPath)
        assertEquals(2, aliases.size)
    }

    @Test
    fun `dependencyAliases returns empty list when manifest has no dependencies section`() {
        val resolver = FakeManifestResolver(aliases = emptyList())
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val aliases = manifest.dependencyAliases()

        assertNotNull(aliases)
        assertEquals(0, aliases.size)
    }

    @Test
    fun `resolveDependency propagates missing actr_type error`() {
        val resolver =
            FakeManifestResolver().apply {
                dependencyErrors["BadDep"] = ActrException.Config("Dependency 'BadDep' has no actr_type")
            }
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val err =
            assertFailsWith<ActrException.Config> {
                manifest.resolveDependency("BadDep")
            }

        assertTrue("actr_type" in err.message.orEmpty())
    }

    @Test
    fun `resolveDependency propagates malformed manifest error`() {
        val resolver =
            FakeManifestResolver().apply {
                dependencyErrors["anything"] = ActrException.Config("Failed to parse manifest")
            }
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val err =
            assertFailsWith<ActrException.Config> {
                manifest.resolveDependency("anything")
            }

        assertTrue("parse" in err.message.orEmpty())
    }

    @Test
    fun `aliasList propagates malformed manifest error`() {
        val resolver =
            FakeManifestResolver().apply {
                aliasListError = ActrException.Config("Failed to parse manifest")
            }
        val manifest = Manifest.from(Path.of("/tmp/actr.toml"), resolver)

        val err =
            assertFailsWith<ActrException.Config> {
                manifest.dependencyAliases()
            }

        assertTrue("parse" in err.message.orEmpty())
    }
}
