use super::{LangTemplate, ProjectTemplateName, TemplateContext};
use crate::error::Result;
use std::collections::HashMap;

pub mod data_stream;
pub mod echo;

pub struct SwiftTemplate;

impl LangTemplate for SwiftTemplate {
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
            ProjectTemplateName::DataStream => {
                data_stream::load(&mut files)?;
            }
        }

        Ok(files)
    }
}
