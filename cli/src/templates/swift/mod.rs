use super::{LangTemplate, ProjectTemplateName, TemplateContext};
use crate::error::Result;
use std::collections::HashMap;

pub mod echo;
pub mod empty;

pub struct SwiftTemplate;

impl LangTemplate for SwiftTemplate {
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
            ProjectTemplateName::Empty => {
                empty::load(&mut files)?;
            }
            ProjectTemplateName::DataStream => {
                empty::load(&mut files)?;
            }
        }

        Ok(files)
    }
}
