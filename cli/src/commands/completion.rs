//! `actr completion` — generate shell completion scripts.

use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use anyhow::Result;
use async_trait::async_trait;
use clap::{Args, ValueEnum};
use clap_complete::{generate, shells};
use std::io;

#[derive(Args, Debug)]
#[command(
    about = "Generate shell completion script",
    long_about = "Generate a completion script for the given shell. Pipe the output to your shell's completion directory.\n\nExample:\n  actr completion bash > /usr/local/etc/bash_completion.d/actr\n  actr completion zsh  > \"$fpath[1]/_actr\""
)]
pub struct CompletionCommand {
    /// Target shell
    #[arg(value_enum)]
    pub shell: Shell,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
    Elvish,
}

#[async_trait]
impl Command for CompletionCommand {
    async fn execute(&self, _ctx: &CommandContext) -> Result<CommandResult> {
        // We need the top-level Cli struct to generate completions.
        // main.rs re-exports this via a helper.
        let mut cmd = crate::cli::build_cli();
        let bin = "actr".to_string();
        let mut out = io::stdout();
        match self.shell {
            Shell::Bash => generate(shells::Bash, &mut cmd, bin, &mut out),
            Shell::Zsh => generate(shells::Zsh, &mut cmd, bin, &mut out),
            Shell::Fish => generate(shells::Fish, &mut cmd, bin, &mut out),
            Shell::PowerShell => generate(shells::PowerShell, &mut cmd, bin, &mut out),
            Shell::Elvish => generate(shells::Elvish, &mut cmd, bin, &mut out),
        }
        Ok(CommandResult::Success(String::new()))
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "completion"
    }

    fn description(&self) -> &str {
        "Generate shell completion script"
    }
}
