//! TypeScript 生成测试

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn test_proto_type_to_typescript() {
        assert_eq!(super::super::proto_type_to_typescript("string"), "string");
        assert_eq!(super::super::proto_type_to_typescript("int32"), "number");
        assert_eq!(super::super::proto_type_to_typescript("uint64"), "number");
        assert_eq!(super::super::proto_type_to_typescript("bool"), "boolean");
        assert_eq!(
            super::super::proto_type_to_typescript("bytes"),
            "Uint8Array"
        );
        assert_eq!(
            super::super::proto_type_to_typescript("CustomType"),
            "CustomType"
        );
    }

    #[test]
    fn test_generate_ts_message_type() {
        use crate::{ProtoField, ProtoMessage};

        let message = ProtoMessage {
            name: "TestMessage".to_string(),
            fields: vec![
                ProtoField {
                    name: "name".to_string(),
                    field_type: "string".to_string(),
                    number: 1,
                    is_repeated: false,
                    is_optional: false,
                },
                ProtoField {
                    name: "tags".to_string(),
                    field_type: "string".to_string(),
                    number: 2,
                    is_repeated: true,
                    is_optional: false,
                },
                ProtoField {
                    name: "count".to_string(),
                    field_type: "int32".to_string(),
                    number: 3,
                    is_repeated: false,
                    is_optional: true,
                },
            ],
        };

        let ts_type = super::super::generate_ts_message_type(&message);

        assert!(ts_type.contains("export interface TestMessage"));
        assert!(ts_type.contains("name: string;"));
        assert!(ts_type.contains("tags: string[];"));
        assert!(ts_type.contains("count?: number;"));
    }

    #[test]
    fn test_generate_ts_actor_ref_method() {
        use crate::ProtoMethod;

        let method = ProtoMethod {
            name: "GetUser".to_string(),
            input_type: "GetUserRequest".to_string(),
            output_type: "GetUserResponse".to_string(),
            is_streaming: false,
        };

        let ts_method = super::super::generate_ts_actor_ref_method(&method, "UserService");

        assert!(ts_method.contains("async getUser"));
        assert!(ts_method.contains("request: GetUserRequest"));
        assert!(ts_method.contains("Promise<GetUserResponse>"));
        assert!(ts_method.contains("this.callRaw('UserService:GetUser'"));
    }

    #[test]
    fn test_generate_ts_actor_ref_method_streaming() {
        use crate::ProtoMethod;

        let method = ProtoMethod {
            name: "StreamData".to_string(),
            input_type: "StreamRequest".to_string(),
            output_type: "StreamResponse".to_string(),
            is_streaming: true,
        };

        let ts_method = super::super::generate_ts_actor_ref_method(&method, "DataService");

        assert!(ts_method.contains("subscribeStreamData"));
        assert!(ts_method.contains("callback: (data: StreamResponse) => void"));
        assert!(ts_method.contains("this.subscribe('DataService:StreamData'"));
    }
}
