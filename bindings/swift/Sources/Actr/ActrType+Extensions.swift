import Foundation

public extension ActrType {
    /// Returns a string representation of the actor type in the format "manufacturer+name".
    ///
    /// Example: `ActrType(manufacturer: "acme", name: "EchoService").toStringRepr()` returns `"acme+EchoService"`
    func toStringRepr() -> String {
        return "\(manufacturer)+\(name)"
    }

    /// Creates an `ActrType` from a string representation in the format "manufacturer+name".
    ///
    /// - Parameter stringRepr: String representation in the format "manufacturer+name" (e.g., "acme+EchoService")
    /// - Returns: An `ActrType` instance
    /// - Throws: `ActrError.ConfigError` if the string format is invalid or contains invalid characters
    ///
    /// Example:
    /// ```swift
    /// let type = try ActrType.fromStringRepr("acme+EchoService")
    /// // type.manufacturer == "acme"
    /// // type.name == "EchoService"
    /// ```
    static func fromStringRepr(_ stringRepr: String) throws -> ActrType {
        guard let plusIndex = stringRepr.firstIndex(of: "+") else {
            throw ActrError.ConfigError(msg: "Invalid ActrType format: '\(stringRepr)'. Expected format: manufacturer+name (e.g., acme+EchoService)")
        }

        let manufacturer = String(stringRepr[..<plusIndex])
        let name = String(stringRepr[stringRepr.index(after: plusIndex)...])

        // Validate that manufacturer and name are not empty
        guard !manufacturer.isEmpty else {
            throw ActrError.ConfigError(msg: "Invalid manufacturer: manufacturer cannot be empty")
        }

        guard !name.isEmpty else {
            throw ActrError.ConfigError(msg: "Invalid type name: name cannot be empty")
        }

        // Basic validation: manufacturer and name should not contain invalid characters
        // This is a simplified validation. For stricter validation matching Rust's Name validation,
        // you may need to add more checks based on the Name validation rules.
        let invalidChars = CharacterSet(charactersIn: "+@:")
        if manufacturer.rangeOfCharacter(from: invalidChars) != nil {
            throw ActrError.ConfigError(msg: "Invalid manufacturer: '\(manufacturer)' contains invalid characters")
        }

        if name.rangeOfCharacter(from: invalidChars) != nil {
            throw ActrError.ConfigError(msg: "Invalid type name: '\(name)' contains invalid characters")
        }

        return ActrType(manufacturer: manufacturer, name: name)
    }
}
