use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "fixtures/"]
pub struct FixtureAssets;
