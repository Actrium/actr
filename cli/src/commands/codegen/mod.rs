mod kotlin;
mod metadata;
mod proto_model;
mod python;
mod rust;
mod scaffold;
mod swift;
mod traits;
mod typescript;

pub use crate::commands::SupportedLanguage;
use crate::error::Result;
use kotlin::KotlinGenerator;
pub use metadata::{
    ACTR_GEN_META_FILE, ActrGenMetadata, load_metadata, metadata_path, write_metadata,
};
pub use proto_model::{MethodModel, ProtoFileModel, ProtoModel, ProtoSide, ServiceModel};
use python::PythonGenerator;
use rust::RustGenerator;
pub use scaffold::{ScaffoldCatalog, ScaffoldMethod, ScaffoldService};
use swift::SwiftGenerator;
use tracing::info;
pub use traits::{GenContext, LanguageGenerator, ScaffoldType};
use typescript::TypeScriptGenerator;

pub struct GeneratorFactory;

impl GeneratorFactory {
    pub fn get_generator(language: SupportedLanguage) -> Box<dyn LanguageGenerator> {
        match language {
            SupportedLanguage::Rust => Box::new(RustGenerator),
            SupportedLanguage::Python => Box::new(PythonGenerator),
            SupportedLanguage::Swift => Box::new(SwiftGenerator),
            SupportedLanguage::Kotlin => Box::new(KotlinGenerator),
            SupportedLanguage::TypeScript => Box::new(TypeScriptGenerator),
        }
    }
}

pub async fn execute_codegen(language: SupportedLanguage, context: &GenContext) -> Result<()> {
    let generator = GeneratorFactory::get_generator(language);

    let mut all_files = generator.generate_infrastructure(context).await?;
    if !context.no_scaffold {
        all_files.extend(generator.generate_scaffold(context).await?);
    }
    if !context.no_format {
        generator.format_code(context, &all_files).await?;
    }

    generator.validate_code(context).await?;

    info!("Code generation completed");

    generator.finalize_generation(context).await?;

    generator.print_next_steps(context);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_factory_returns_all_languages() {
        for language in [
            SupportedLanguage::Rust,
            SupportedLanguage::Python,
            SupportedLanguage::Swift,
            SupportedLanguage::Kotlin,
            SupportedLanguage::TypeScript,
        ] {
            let _ = GeneratorFactory::get_generator(language);
        }
    }
}
