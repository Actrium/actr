use super::*;

#[test]
fn skips_validation_when_requested() {
    assert!(!should_validate(true));
}

#[test]
fn runs_validation_by_default() {
    assert!(should_validate(false));
}

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
