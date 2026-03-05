pub mod echo;

pub use echo::load;

use super::{LangTemplate, ProjectTemplateName, TemplateContext};
use crate::error::Result;
use std::collections::HashMap;

pub struct RustTemplate;

impl LangTemplate for RustTemplate {
    fn load_files(
        &self,
        template_name: ProjectTemplateName,
        context: &TemplateContext,
    ) -> Result<HashMap<String, String>> {
        let mut files = HashMap::new();

        match template_name {
            ProjectTemplateName::Echo => {
                echo::load(&mut files, context.is_service)?;
            }
            ProjectTemplateName::DataStream => {
                return Err(crate::error::ActrCliError::Unsupported(
                    "DataStream template is not supported for Rust yet".to_string(),
                ));
            }
        }

        Ok(files)
    }
}
