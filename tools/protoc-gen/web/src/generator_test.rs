//! Proto 解析测试

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn test_extract_package() {
        let content = r#"
            syntax = "proto3";
            package echo.v1;

            service Echo {
                rpc Hello(HelloRequest) returns (HelloResponse);
            }
        "#;

        let package = super::super::extract_package(content);
        assert_eq!(package, "echo.v1");
    }

    #[test]
    fn test_extract_service_name() {
        let content = r#"
            service Echo {
                rpc Hello(HelloRequest) returns (HelloResponse);
            }
        "#;

        let service_name = super::super::extract_service_name(content);
        assert_eq!(service_name, Some("Echo".to_string()));
    }

    #[test]
    fn test_parse_rpc_method() {
        let line = "rpc Hello(HelloRequest) returns (HelloResponse);";
        let method = super::super::parse_rpc_method(line).unwrap();

        assert_eq!(method.name, "Hello");
        assert_eq!(method.input_type, "HelloRequest");
        assert_eq!(method.output_type, "HelloResponse");
        assert!(!method.is_streaming);
    }

    #[test]
    fn test_parse_rpc_method_streaming() {
        let line = "rpc StreamData(stream DataRequest) returns (stream DataResponse);";
        let method = super::super::parse_rpc_method(line).unwrap();

        assert_eq!(method.name, "StreamData");
        assert_eq!(method.input_type, "DataRequest");
        assert_eq!(method.output_type, "DataResponse");
        assert!(method.is_streaming);
    }

    #[test]
    fn test_parse_message_field() {
        let line = "string name = 1;";
        let field = super::super::parse_message_field(line).unwrap();

        assert_eq!(field.name, "name");
        assert_eq!(field.field_type, "string");
        assert!(!field.is_repeated);
        assert!(!field.is_optional);
    }

    #[test]
    fn test_parse_message_field_repeated() {
        let line = "repeated string items = 2;";
        let field = super::super::parse_message_field(line).unwrap();

        assert_eq!(field.name, "items");
        assert_eq!(field.field_type, "string");
        assert!(field.is_repeated);
        assert!(!field.is_optional);
    }

    #[test]
    fn test_parse_message_field_optional() {
        let line = "optional int32 count = 3;";
        let field = super::super::parse_message_field(line).unwrap();

        assert_eq!(field.name, "count");
        assert_eq!(field.field_type, "int32");
        assert!(!field.is_repeated);
        assert!(field.is_optional);
    }

    #[test]
    fn test_proto_type_to_rust() {
        assert_eq!(super::super::proto_type_to_rust("string"), "String");
        assert_eq!(super::super::proto_type_to_rust("int32"), "i32");
        assert_eq!(super::super::proto_type_to_rust("uint64"), "u64");
        assert_eq!(super::super::proto_type_to_rust("bool"), "bool");
        assert_eq!(super::super::proto_type_to_rust("bytes"), "Vec<u8>");
        assert_eq!(super::super::proto_type_to_rust("CustomType"), "CustomType");
    }

    #[test]
    fn test_extract_methods() {
        let content = r#"
            service Echo {
                rpc Hello(HelloRequest) returns (HelloResponse);
                rpc StreamData(stream DataRequest) returns (stream DataResponse);
            }
        "#;

        let methods = super::super::extract_methods(content, "Echo");
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].name, "Hello");
        assert_eq!(methods[1].name, "StreamData");
    }

    #[test]
    fn test_extract_messages() {
        let content = r#"
            message HelloRequest {
                string name = 1;
                int32 age = 2;
            }

            message HelloResponse {
                string message = 1;
            }
        "#;

        let messages = super::super::extract_messages(content);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].name, "HelloRequest");
        assert_eq!(messages[0].fields.len(), 2);
        assert_eq!(messages[1].name, "HelloResponse");
        assert_eq!(messages[1].fields.len(), 1);
    }
}
