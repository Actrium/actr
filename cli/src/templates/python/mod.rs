pub mod echo;

use super::{LangTemplate, ProjectTemplateName, TemplateContext};
use crate::error::Result;
use std::collections::HashMap;

pub struct PythonTemplate;

impl LangTemplate for PythonTemplate {
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
                return Err(crate::error::ActrCliError::Unsupported(
                    "DataStream template is not supported for Python yet".to_string(),
                ));
            }
        }

        Ok(files)
    }
}
