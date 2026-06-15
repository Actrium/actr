use super::{LangTemplate, ProjectTemplateName, TemplateContext};
use crate::error::Result;
use std::collections::HashMap;

pub mod data_stream;
pub mod echo;

pub struct KotlinTemplate;

impl LangTemplate for KotlinTemplate {
    fn load_files(
        &self,
        template_name: ProjectTemplateName,
        _context: &TemplateContext,
    ) -> Result<HashMap<String, String>> {
        let mut files = HashMap::new();

        match template_name {
            ProjectTemplateName::Echo => {
                echo::load(&mut files)?;
            }
            ProjectTemplateName::Empty => {
                return Err(crate::error::ActrCliError::Unsupported(
                    "Empty template is not supported for Kotlin yet".to_string(),
                ));
            }
            ProjectTemplateName::DataStream => {
                data_stream::load(&mut files)?;
            }
        }

        Ok(files)
    }
}
