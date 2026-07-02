use super::*;

#[test]
fn factory_returns_all_languages() {
    for lang in [
        SupportedLanguage::Rust,
        SupportedLanguage::Python,
        SupportedLanguage::Swift,
        SupportedLanguage::Kotlin,
        SupportedLanguage::TypeScript,
    ] {
        let _ = InitializerFactory::get_initializer(lang).unwrap();
    }
}
