//! Configuration types.

use std::path::PathBuf;

/// Configuration for web code generation.
#[derive(Debug, Clone)]
pub struct WebCodegenConfig {
    /// List of proto file paths.
    pub proto_files: Vec<PathBuf>,

    /// Rust output directory for the WASM side.
    pub rust_output_dir: PathBuf,

    /// TypeScript output directory for the web SDK side.
    pub ts_output_dir: PathBuf,

    /// Whether to generate React Hooks.
    pub generate_react_hooks: bool,

    /// Proto include paths used to resolve imports.
    pub includes: Vec<PathBuf>,

    /// Whether to format generated code.
    pub format_code: bool,

    /// Optional custom template directory.
    pub custom_templates_dir: Option<PathBuf>,
}

impl WebCodegenConfig {
    /// Create a new configuration builder.
    pub fn builder() -> WebCodegenConfigBuilder {
        WebCodegenConfigBuilder::default()
    }

    /// Validate the configuration.
    pub fn validate(&self) -> crate::Result<()> {
        use crate::error::CodegenError;

        // At least one proto file is required.
        if self.proto_files.is_empty() {
            return Err(CodegenError::config("at least one proto file is required"));
        }

        // Every proto file must exist.
        for proto in &self.proto_files {
            if !proto.exists() {
                return Err(CodegenError::FileNotFound(proto.clone()));
            }
        }

        // Every include directory must exist.
        for include in &self.includes {
            if !include.exists() {
                return Err(CodegenError::FileNotFound(include.clone()));
            }
        }

        Ok(())
    }
}

/// Configuration builder.
#[derive(Default)]
pub struct WebCodegenConfigBuilder {
    proto_files: Vec<PathBuf>,
    rust_output_dir: Option<PathBuf>,
    ts_output_dir: Option<PathBuf>,
    generate_react_hooks: bool,
    includes: Vec<PathBuf>,
    format_code: bool,
    custom_templates_dir: Option<PathBuf>,
}

impl WebCodegenConfigBuilder {
    /// Add one proto file.
    pub fn proto_file<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.proto_files.push(path.into());
        self
    }

    /// Add multiple proto files.
    pub fn proto_files<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        self.proto_files.extend(paths.into_iter().map(Into::into));
        self
    }

    /// Set the Rust output directory.
    pub fn rust_output<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.rust_output_dir = Some(dir.into());
        self
    }

    /// Set the TypeScript output directory.
    pub fn ts_output<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.ts_output_dir = Some(dir.into());
        self
    }

    /// Enable React Hooks generation.
    pub fn with_react_hooks(mut self, enabled: bool) -> Self {
        self.generate_react_hooks = enabled;
        self
    }

    /// Add one include path.
    pub fn include<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.includes.push(path.into());
        self
    }

    /// Add multiple include paths.
    pub fn includes<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        self.includes.extend(paths.into_iter().map(Into::into));
        self
    }

    /// Enable code formatting.
    pub fn with_formatting(mut self, enabled: bool) -> Self {
        self.format_code = enabled;
        self
    }

    /// Set a custom template directory.
    pub fn custom_templates<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.custom_templates_dir = Some(dir.into());
        self
    }

    /// Build the configuration.
    pub fn build(self) -> crate::Result<WebCodegenConfig> {
        use crate::error::CodegenError;

        let rust_output_dir = self
            .rust_output_dir
            .ok_or_else(|| CodegenError::config("missing rust_output_dir configuration"))?;

        let ts_output_dir = self
            .ts_output_dir
            .ok_or_else(|| CodegenError::config("missing ts_output_dir configuration"))?;

        let config = WebCodegenConfig {
            proto_files: self.proto_files,
            rust_output_dir,
            ts_output_dir,
            generate_react_hooks: self.generate_react_hooks,
            includes: self.includes,
            format_code: self.format_code,
            custom_templates_dir: self.custom_templates_dir,
        };

        config.validate()?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder() {
        let config = WebCodegenConfig::builder()
            .proto_file("test.proto")
            .rust_output("src/generated")
            .ts_output("src/types")
            .with_react_hooks(true)
            .include("proto")
            .with_formatting(true);

        // `build()` is not used here because the files do not exist in the test.
        assert!(config.generate_react_hooks);
        assert!(config.format_code);
    }
}
