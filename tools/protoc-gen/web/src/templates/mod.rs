//! 代码生成模板

use handlebars::Handlebars;

/// 初始化模板引擎
#[allow(dead_code)]
pub fn init_templates() -> Handlebars<'static> {
    // 注册内置模板
    // TODO: 添加具体的模板

    Handlebars::new()
}

/// 内置的 Rust Actor 模板
#[allow(dead_code)]
pub const RUST_ACTOR_TEMPLATE: &str = r#"
//! 自动生成的 Actor 代码
//! 服务: {{service_name}}
//! 包: {{package}}
//!
//! ⚠️  请勿手动编辑此文件

use wasm_bindgen::prelude::*;
use serde::{Serialize, Deserialize};

/// {{service_name}} Actor
#[wasm_bindgen]
pub struct {{service_name}}Actor {
    // Actor 状态
}

#[wasm_bindgen]
impl {{service_name}}Actor {
    /// 创建新的 Actor 实例
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {}
    }

    {{#each methods}}
    /// {{this.name}} 方法
    pub async fn {{this.name}}(&self, request: {{this.input_type}}) -> Result<{{this.output_type}}, JsValue> {
        // TODO: 实现方法逻辑
        unimplemented!()
    }
    {{/each}}
}
"#;

/// 内置的 TypeScript 类型模板
#[allow(dead_code)]
pub const TS_TYPES_TEMPLATE: &str = r#"
/**
 * 自动生成的类型定义
 * 服务: {{service_name}}
 * 包: {{package}}
 *
 * ⚠️  请勿手动编辑此文件
 */

{{#each messages}}
export interface {{this.name}} {
  {{#each this.fields}}
  {{this.name}}{{#if this.is_optional}}?{{/if}}: {{this.field_type}}{{#if this.is_repeated}}[]{{/if}};
  {{/each}}
}
{{/each}}
"#;

/// 内置的 ActorRef 模板
#[allow(dead_code)]
pub const ACTOR_REF_TEMPLATE: &str = r#"
/**
 * 自动生成的 ActorRef 包装
 * 服务: {{service_name}}
 *
 * ⚠️  请勿手动编辑此文件
 */

import { ActorRef } from '@actr/web';
import type { {{#each messages}}{{this.name}}, {{/each}} } from './{{file_name}}.types';

/**
 * {{service_name}} Actor 引用
 */
export class {{service_name}}ActorRef extends ActorRef {
  /**
   * 创建新的 ActorRef 实例
   */
  constructor(actorId: string) {
    super(actorId);
  }

  {{#each methods}}
  /**
   * {{this.name}} 方法
   */
  async {{this.name}}(request: {{this.input_type}}): Promise<{{this.output_type}}> {
    return this.call('{{../service_name}}', '{{this.name}}', request);
  }
  {{/each}}

  {{#each streams}}
  /**
   * 订阅 {{this.name}} 流
   */
  subscribe{{this.name}}(callback: (data: {{this.stream_type}}) => void): () => void {
    return this.subscribe('{{this.topic}}', callback);
  }
  {{/each}}
}
"#;
