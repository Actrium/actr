pub mod echo;

use super::{LangTemplate, ProjectTemplateName, TemplateContext};
use crate::error::Result;
use std::collections::HashMap;

pub struct WebTemplate;

impl LangTemplate for WebTemplate {
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
                    "DataStream template is not supported for Web yet".to_string(),
                ));
            }
        }

        Ok(files)
    }
}
