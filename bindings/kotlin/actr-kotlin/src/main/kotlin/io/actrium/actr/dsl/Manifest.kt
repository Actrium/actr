/** Manifest resolution helpers for Kotlin/Android. */
package io.actrium.actr.dsl

import io.actrium.actr.ActrException
import io.actrium.actr.ActrType
import io.actrium.actr.resolveManifestDependency as ffiResolveManifestDependency
import io.actrium.actr.resolveManifestDependencyAliasList as ffiResolveManifestDependencyAliasList
import io.actrium.actr.resolveManifestPackageActrType as ffiResolveManifestPackageActrType
import java.io.File
import java.nio.file.Path
import kotlin.io.path.pathString

// ============================================================================
// Manifest class — Kotlin-idiomatic entry point
// ============================================================================

internal interface ManifestResolver {
    @Throws(ActrException::class)
    fun packageType(manifestPath: String): ActrType

    @Throws(ActrException::class)
    fun resolveDependency(
        manifestPath: String,
        dependencyAlias: String,
    ): ActrType

    @Throws(ActrException::class)
    fun dependencyAliases(manifestPath: String): List<String>
}

internal object FfiManifestResolver : ManifestResolver {
    override fun packageType(manifestPath: String): ActrType =
        ffiResolveManifestPackageActrType(manifestPath)

    override fun resolveDependency(
        manifestPath: String,
        dependencyAlias: String,
    ): ActrType =
        ffiResolveManifestDependency(manifestPath, dependencyAlias)

    override fun dependencyAliases(manifestPath: String): List<String> =
        ffiResolveManifestDependencyAliasList(manifestPath)
}

/**
 * A parsed manifest.toml file that provides typed access to package identity
 * and dependency resolution.
 *
 * This is the recommended Kotlin entry point for manifest operations.
 * Construct with a [Path], [File], or raw path [String], then query
 * package type, dependency aliases, and resolved dependency types.
 *
 * Example:
 * ```kotlin
 * val manifest = Manifest.from(Path.of("/app/actr.toml"))
 * val myType: ActrType = manifest.packageType()
 * val aliases: List<String> = manifest.dependencyAliases()
 * val echoType: ActrType = manifest.resolveDependency("EchoService")
 * ```
 */
class Manifest internal constructor(
    private val manifestPath: String,
    private val resolver: ManifestResolver,
) {
    constructor(manifestPath: String) : this(manifestPath, FfiManifestResolver)

    /** Resolve the package's own [ActrType] from the `[package]` block. */
    @Throws(ActrException::class)
    fun packageType(): ActrType = resolver.packageType(manifestPath)

    /** Resolve a dependency's [ActrType] by its alias. */
    @Throws(ActrException::class)
    fun resolveDependency(alias: String): ActrType =
        resolver.resolveDependency(manifestPath, alias)

    /** List all dependency aliases declared in the manifest. */
    @Throws(ActrException::class)
    fun dependencyAliases(): List<String> = resolver.dependencyAliases(manifestPath)

    companion object {
        /** Create a [Manifest] from a [Path]. */
        fun from(path: Path): Manifest = Manifest(path.pathString)

        /** Create a [Manifest] from a [File]. */
        fun from(file: File): Manifest = Manifest(file.absolutePath)

        internal fun from(
            path: Path,
            resolver: ManifestResolver,
        ): Manifest = Manifest(path.pathString, resolver)

        internal fun from(
            file: File,
            resolver: ManifestResolver,
        ): Manifest = Manifest(file.absolutePath, resolver)
    }
}

// ============================================================================
// Top-level convenience functions with Path / File overloads
// ============================================================================

/**
 * Resolve a dependency's [ActrType] from a manifest.toml file.
 *
 * Kotlin-idiomatic overload accepting a [Path].
 */
@Throws(io.actrium.actr.ActrException::class)
fun resolveManifestDependency(
    manifestPath: Path,
    dependencyAlias: String,
): ActrType = resolveManifestDependency(manifestPath, dependencyAlias, FfiManifestResolver)

internal fun resolveManifestDependency(
    manifestPath: Path,
    dependencyAlias: String,
    resolver: ManifestResolver,
): ActrType = resolver.resolveDependency(manifestPath.pathString, dependencyAlias)

/**
 * Resolve a dependency's [ActrType] from a manifest.toml file.
 *
 * Kotlin-idiomatic overload accepting a [File].
 */
@Throws(io.actrium.actr.ActrException::class)
fun resolveManifestDependency(
    manifestFile: File,
    dependencyAlias: String,
): ActrType = resolveManifestDependency(manifestFile, dependencyAlias, FfiManifestResolver)

internal fun resolveManifestDependency(
    manifestFile: File,
    dependencyAlias: String,
    resolver: ManifestResolver,
): ActrType = resolver.resolveDependency(manifestFile.absolutePath, dependencyAlias)

/**
 * List all dependency aliases from a manifest.toml file.
 *
 * Kotlin-idiomatic overload accepting a [Path].
 */
@Throws(io.actrium.actr.ActrException::class)
fun resolveManifestDependencyAliasList(manifestPath: Path): List<String> =
    resolveManifestDependencyAliasList(manifestPath, FfiManifestResolver)

internal fun resolveManifestDependencyAliasList(
    manifestPath: Path,
    resolver: ManifestResolver,
): List<String> = resolver.dependencyAliases(manifestPath.pathString)

/**
 * List all dependency aliases from a manifest.toml file.
 *
 * Kotlin-idiomatic overload accepting a [File].
 */
@Throws(io.actrium.actr.ActrException::class)
fun resolveManifestDependencyAliasList(manifestFile: File): List<String> =
    resolveManifestDependencyAliasList(manifestFile, FfiManifestResolver)

internal fun resolveManifestDependencyAliasList(
    manifestFile: File,
    resolver: ManifestResolver,
): List<String> = resolver.dependencyAliases(manifestFile.absolutePath)

/**
 * Resolve the package's own [ActrType] from a manifest.toml file.
 *
 * Kotlin-idiomatic overload accepting a [Path].
 */
@Throws(io.actrium.actr.ActrException::class)
fun resolveManifestPackageActrType(manifestPath: Path): ActrType =
    resolveManifestPackageActrType(manifestPath, FfiManifestResolver)

internal fun resolveManifestPackageActrType(
    manifestPath: Path,
    resolver: ManifestResolver,
): ActrType = resolver.packageType(manifestPath.pathString)

/**
 * Resolve the package's own [ActrType] from a manifest.toml file.
 *
 * Kotlin-idiomatic overload accepting a [File].
 */
@Throws(io.actrium.actr.ActrException::class)
fun resolveManifestPackageActrType(manifestFile: File): ActrType =
    resolveManifestPackageActrType(manifestFile, FfiManifestResolver)

internal fun resolveManifestPackageActrType(
    manifestFile: File,
    resolver: ManifestResolver,
): ActrType = resolver.packageType(manifestFile.absolutePath)
