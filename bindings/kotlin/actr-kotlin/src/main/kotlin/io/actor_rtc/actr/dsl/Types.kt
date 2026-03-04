/** Type builders and extensions for Actor-RTC types. */
package io.actor_rtc.actr.dsl

import io.actor_rtc.actr.ActrId
import io.actor_rtc.actr.ActrType
import io.actor_rtc.actr.DataStream
import io.actor_rtc.actr.MetadataEntry
import io.actor_rtc.actr.Realm

// ============================================================================
// ActrType Builders
// ============================================================================

/**
 * Create an ActrType from a string in "manufacturer:name" format.
 *
 * @param typeString Type string like "acme:EchoService"
 * @return ActrType with parsed manufacturer and name
 * @throws IllegalArgumentException if format is invalid
 */
fun String.toActrType(): ActrType {
    val parts = this.split(":", limit = 2)
    require(parts.size == 2) { "ActrType string must be in 'manufacturer:name' format, got: $this" }
    return ActrType(manufacturer = parts[0], name = parts[1])
}

/**
 * Create an ActrType with DSL syntax.
 *
 * Example:
 * ```kotlin
 * val type = actrType {
 *     manufacturer = "acme"
 *     name = "EchoService"
 * }
 * ```
 */
inline fun actrType(builder: ActrTypeBuilder.() -> Unit): ActrType {
    return ActrTypeBuilder().apply(builder).build()
}

/**
 * Create an ActrType from manufacturer and name.
 *
 * Example:
 * ```kotlin
 * val type = actrType("acme", "EchoService")
 * ```
 */
fun actrType(manufacturer: String, name: String): ActrType {
    return ActrType(manufacturer = manufacturer, name = name)
}

/** Builder for ActrType. */
class ActrTypeBuilder {
    var manufacturer: String = ""
    var name: String = ""

    fun build(): ActrType {
        require(manufacturer.isNotBlank()) { "manufacturer must not be blank" }
        require(name.isNotBlank()) { "name must not be blank" }
        return ActrType(manufacturer = manufacturer, name = name)
    }
}

// ============================================================================
// ActrType Extensions
// ============================================================================

/** Convert ActrType to string representation. */
fun ActrType.toTypeString(): String = "$manufacturer:$name"

/** Check if this type matches a type string. */
fun ActrType.matches(typeString: String): Boolean {
    val other = typeString.toActrType()
    return manufacturer == other.manufacturer && name == other.name
}

// ============================================================================
// ActrId Builders
// ============================================================================

/**
 * Create an ActrId with DSL syntax.
 *
 * Example:
 * ```kotlin
 * val id = actrId {
 *     realm = 2281844430u
 *     serialNumber = 12345uL
 *     type = "acme:EchoService"
 * }
 * ```
 */
inline fun actrId(builder: ActrIdBuilder.() -> Unit): ActrId {
    return ActrIdBuilder().apply(builder).build()
}

/** Builder for ActrId. */
class ActrIdBuilder {
    var realm: UInt = 0u
    var serialNumber: ULong = 0uL
    private var _type: ActrType? = null

    /** Set the actor type from a string. */
    var type: String
        get() = _type?.toTypeString() ?: ""
        set(value) {
            _type = value.toActrType()
        }

    /** Set the actor type directly. */
    fun type(actrType: ActrType) {
        _type = actrType
    }

    /** Set the actor type with manufacturer and name. */
    fun type(manufacturer: String, name: String) {
        _type = ActrType(manufacturer = manufacturer, name = name)
    }

    fun build(): ActrId {
        require(realm > 0u) { "realm must be set" }
        requireNotNull(_type) { "type must be set" }
        return ActrId(realm = Realm(realmId = realm), serialNumber = serialNumber, type = _type!!)
    }
}

// ============================================================================
// ActrId Extensions
// ============================================================================

/** Get the realm ID. */
val ActrId.realmId: UInt
    get() = realm.realmId

/** Get a short string representation. */
fun ActrId.toShortString(): String {
    return "${type.manufacturer}:${type.name}@${serialNumber.toString(16)}"
}

/** Get a full string representation. */
fun ActrId.toFullString(): String {
    return "${type.manufacturer}:${type.name}@${serialNumber.toString(16)}:${realm.realmId}"
}

// ============================================================================
// DataStream Builders
// ============================================================================

/**
 * Create a DataStream with DSL syntax.
 *
 * Example:
 * ```kotlin
 * val stream = dataStream {
 *     streamId = "file-transfer-001"
 *     sequence = 0uL
 *     payload = fileBytes
 *     timestamp = System.currentTimeMillis()
 *     metadata {
 *         "content-type" to "application/octet-stream"
 *         "filename" to "example.txt"
 *     }
 * }
 * ```
 */
inline fun dataStream(builder: DataStreamBuilder.() -> Unit): DataStream {
    return DataStreamBuilder().apply(builder).build()
}

/** Builder for DataStream. */
class DataStreamBuilder {
    var streamId: String = ""
    var sequence: ULong = 0uL
    var payload: ByteArray = ByteArray(0)
    var timestamp: Long? = null
    private val metadataEntries = mutableListOf<MetadataEntry>()

    /**
     * Add metadata entries using DSL.
     *
     * Example:
     * ```kotlin
     * metadata {
     *     "key1" to "value1"
     *     "key2" to "value2"
     * }
     * ```
     */
    fun metadata(builder: MetadataBuilder.() -> Unit) {
        metadataEntries.addAll(MetadataBuilder().apply(builder).entries)
    }

    /** Add a single metadata entry. */
    fun addMetadata(key: String, value: String) {
        metadataEntries.add(MetadataEntry(key = key, value = value))
    }

    /** Set payload from a string. */
    fun payload(text: String, charset: java.nio.charset.Charset = Charsets.UTF_8) {
        payload = text.toByteArray(charset)
    }

    fun build(): DataStream {
        require(streamId.isNotBlank()) { "streamId must not be blank" }
        return DataStream(
                streamId = streamId,
                sequence = sequence,
                payload = payload,
                metadata = metadataEntries.toList(),
                timestampMs = timestamp
        )
    }
}

/** Builder for metadata entries. */
class MetadataBuilder {
    internal val entries = mutableListOf<MetadataEntry>()

    /** Add a metadata entry using infix notation. */
    infix fun String.to(value: String) {
        entries.add(MetadataEntry(key = this, value = value))
    }
}

// ============================================================================
// DataStream Extensions
// ============================================================================

/** Get a metadata value by key. */
fun DataStream.getMetadata(key: String): String? {
    return metadata.find { it.key == key }?.value
}

/** Check if metadata contains a key. */
fun DataStream.hasMetadata(key: String): Boolean {
    return metadata.any { it.key == key }
}

/** Get metadata as a Map. */
fun DataStream.metadataMap(): Map<String, String> {
    return metadata.associate { it.key to it.value }
}

// ============================================================================
// Realm Builders
// ============================================================================

/** Create a Realm from a realm ID. */
fun realm(id: UInt): Realm = Realm(realmId = id)

/** Create a Realm from an Int. */
fun realm(id: Int): Realm = Realm(realmId = id.toUInt())
