use std::{fmt, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedProjectLanguage {
    Rust,
    Swift,
    Kotlin,
    Python,
    TypeScript,
    Ambiguous,
    Unknown,
}

impl DetectedProjectLanguage {
    pub fn detect(root: &Path) -> Self {
        let has_swift = root.join("Package.swift").exists() || root.join("project.yml").exists();
        let has_kotlin =
            root.join("build.gradle.kts").exists() || root.join("build.gradle").exists();
        let has_python =
            root.join("pyproject.toml").exists() || root.join("requirements.txt").exists();
        let has_typescript =
            root.join("tsconfig.json").exists() || root.join("package.json").exists();
        let has_rust = root.join("Cargo.toml").exists();

        let matched_count = [has_swift, has_kotlin, has_python, has_typescript, has_rust]
            .into_iter()
            .filter(|matched| *matched)
            .count();

        match matched_count {
            0 => Self::Unknown,
            1 => {
                if has_swift {
                    Self::Swift
                } else if has_kotlin {
                    Self::Kotlin
                } else if has_python {
                    Self::Python
                } else if has_typescript {
                    Self::TypeScript
                } else {
                    Self::Rust
                }
            }
            _ => Self::Ambiguous,
        }
    }

    pub fn cli_name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Swift => "swift",
            Self::Kotlin => "kotlin",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Ambiguous => "ambiguous",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for DetectedProjectLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.cli_name())
    }
}

#[cfg(test)]
mod tests {
    use super::DetectedProjectLanguage;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn detects_rust_from_cargo_toml() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();

        assert_eq!(
            DetectedProjectLanguage::detect(dir.path()),
            DetectedProjectLanguage::Rust
        );
    }

    #[test]
    fn returns_unknown_without_language_markers() {
        let dir = TempDir::new().unwrap();

        assert_eq!(
            DetectedProjectLanguage::detect(dir.path()),
            DetectedProjectLanguage::Unknown
        );
    }

    #[test]
    fn returns_ambiguous_with_multiple_language_markers() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("package.json"),
            "{\n  \"name\": \"demo\"\n}\n",
        )
        .unwrap();

        assert_eq!(
            DetectedProjectLanguage::detect(dir.path()),
            DetectedProjectLanguage::Ambiguous
        );
    }
}
