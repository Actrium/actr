use crate::commands::SupportedLanguage;
use crate::commands::codegen::metadata::{ActrGenMetadata, load_metadata};
use crate::commands::codegen::proto_model::ProtoModel;
use crate::commands::codegen::traits::GenContext;
use crate::error::Result;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ScaffoldCatalog {
    pub local_services: Vec<ScaffoldService>,
    pub remote_services: Vec<ScaffoldService>,
}

#[derive(Debug, Clone)]
pub struct ScaffoldService {
    pub name: String,
    pub package: String,
    pub proto_file: PathBuf,
    pub handler_interface: Option<String>,
    pub workload_type: Option<String>,
    pub dispatcher_type: Option<String>,
    pub client_type: Option<String>,
    pub actr_type: Option<String>,
    pub methods: Vec<ScaffoldMethod>,
}

#[derive(Debug, Clone)]
pub struct ScaffoldMethod {
    pub name: String,
    pub snake_name: String,
    pub input_type: String,
    pub output_type: String,
    pub route_key: String,
}

impl ScaffoldCatalog {
    pub fn load(context: &GenContext, language: SupportedLanguage) -> Result<Self> {
        let expected_language = language_key(language);
        let metadata = load_metadata(&context.output)?
            .filter(|metadata| metadata.language == expected_language)
            .unwrap_or_else(|| ActrGenMetadata::from_proto_model(language, &context.proto_model));
        Ok(Self::from_metadata(&metadata))
    }

    fn from_metadata(metadata: &ActrGenMetadata) -> Self {
        Self {
            local_services: metadata
                .local_services
                .iter()
                .map(|service| ScaffoldService {
                    name: service.name.clone(),
                    package: service.package.clone(),
                    proto_file: PathBuf::from(&service.proto_file),
                    handler_interface: Some(service.handler_interface.clone()),
                    workload_type: Some(service.workload_type.clone()),
                    dispatcher_type: Some(service.dispatcher_type.clone()),
                    client_type: None,
                    actr_type: None,
                    methods: service
                        .methods
                        .iter()
                        .map(|method| ScaffoldMethod {
                            name: method.name.clone(),
                            snake_name: method.snake_name.clone(),
                            input_type: method.input_type.clone(),
                            output_type: method.output_type.clone(),
                            route_key: method.route_key.clone(),
                        })
                        .collect(),
                })
                .collect(),
            remote_services: metadata
                .remote_services
                .iter()
                .map(|service| ScaffoldService {
                    name: service.name.clone(),
                    package: service.package.clone(),
                    proto_file: PathBuf::from(&service.proto_file),
                    handler_interface: None,
                    workload_type: None,
                    dispatcher_type: None,
                    client_type: Some(service.client_type.clone()),
                    actr_type: Some(service.actr_type.clone()),
                    methods: service
                        .methods
                        .iter()
                        .map(|method| ScaffoldMethod {
                            name: method.name.clone(),
                            snake_name: method.snake_name.clone(),
                            input_type: method.input_type.clone(),
                            output_type: method.output_type.clone(),
                            route_key: method.route_key.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn has_any_methods(&self) -> bool {
        self.local_services
            .iter()
            .any(|service| !service.methods.is_empty())
            || self
                .remote_services
                .iter()
                .any(|service| !service.methods.is_empty())
    }
}

fn language_key(language: SupportedLanguage) -> &'static str {
    match language {
        SupportedLanguage::Rust => "rust",
        SupportedLanguage::Python => "python",
        SupportedLanguage::Swift => "swift",
        SupportedLanguage::Kotlin => "kotlin",
        SupportedLanguage::TypeScript => "typescript",
    }
}

#[allow(dead_code)]
fn _proto_model_is_retained_for_generation_ordering(_proto_model: &ProtoModel) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_metadata_maps_local_and_remote_to_scaffold_services() {
        let metadata = ActrGenMetadata {
            plugin_version: "actr-cli".into(),
            language: "rust".into(),
            local_services: vec![crate::commands::codegen::metadata::LocalServiceMetadata {
                name: "EchoService".into(),
                package: "echo".into(),
                proto_file: "echo.proto".into(),
                handler_interface: "EchoServiceHandler".into(),
                workload_type: "EchoServiceWorkload".into(),
                dispatcher_type: "EchoServiceDispatcher".into(),
                methods: vec![crate::commands::codegen::metadata::MethodMetadata {
                    name: "Echo".into(),
                    snake_name: "echo".into(),
                    input_type: "EchoRequest".into(),
                    output_type: "EchoResponse".into(),
                    route_key: "echo.Echo".into(),
                }],
            }],
            remote_services: vec![],
        };
        let catalog = ScaffoldCatalog::from_metadata(&metadata);
        assert_eq!(catalog.local_services.len(), 1);
        assert_eq!(catalog.local_services[0].name, "EchoService");
        assert!(catalog.local_services[0].handler_interface.is_some());
        assert!(catalog.local_services[0].client_type.is_none());
        assert!(catalog.remote_services.is_empty());
        assert!(catalog.has_any_methods());

        assert!(
            !ScaffoldCatalog {
                local_services: vec![],
                remote_services: vec![]
            }
            .has_any_methods()
        );
    }
}
