//! Command implementations for actr-cli

pub mod init;
pub mod build;
pub mod run;

use crate::error::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Command {
    async fn execute(&self) -> Result<()>;
}