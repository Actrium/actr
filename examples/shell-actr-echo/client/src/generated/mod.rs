//! 自动生成的代码模块
//!
//! 此模块由 `actr gen` 命令自动生成，包括：
//! - protobuf 消息类型定义
//! - Actor 框架代码（路由器、trait）
//!
//! ⚠️  请勿手动修改此目录中的文件

// Protobuf 消息类型（由 prost 生成）
pub mod echo;

// Actor 框架代码（由 protoc-gen-actrframework 生成）
pub mod echo_service_actor;

// 常用类型会在各自的模块中定义，请按需导入
