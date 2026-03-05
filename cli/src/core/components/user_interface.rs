use anyhow::{Context, Result};
use async_trait::async_trait;
use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use std::io::{self, Write};

use crate::core::{ActrCliError, ProgressBar, ServiceInfo, UserInterface};

pub struct ConsoleUI;

impl Default for ConsoleUI {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleUI {
    pub fn new() -> Self {
        Self
    }

    fn read_line(&self) -> Result<String> {
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("Failed to read from stdin")?;
        Ok(input.trim().to_string())
    }
}

#[async_trait]
impl UserInterface for ConsoleUI {
    async fn prompt_input(&self, prompt: &str) -> Result<String> {
        print!("{prompt}: ");
        io::stdout().flush().context("Failed to flush stdout")?;
        self.read_line()
    }

    async fn confirm(&self, message: &str) -> Result<bool> {
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(message)
            .default(false)
            .interact()
            .context("Failed to get confirmation")
    }

    async fn select_from_list(&self, items: &[String], prompt: &str) -> Result<usize> {
        if items.is_empty() {
            return Err(anyhow::anyhow!("Cannot select from an empty list"));
        }

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .items(items)
            .default(0)
            .interact_opt()
            .context("Failed to select from list")?;

        match selection {
            Some(index) => Ok(index),
            None => Err(ActrCliError::OperationCancelled.into()),
        }
    }

    async fn display_service_table(
        &self,
        _items: &[ServiceInfo],
        _headers: &[&str],
        _formatter: fn(&ServiceInfo) -> Vec<String>,
    ) {
        // Discovery command currently implements its own table display
    }

    async fn show_progress(&self, message: &str) -> Result<Box<dyn ProgressBar>> {
        println!("⏳ {message}...");
        Ok(Box::new(ConsoleProgressBar))
    }
}

pub struct ConsoleProgressBar;

impl ProgressBar for ConsoleProgressBar {
    fn update(&self, _progress: f64) {}

    fn set_message(&self, message: &str) {
        println!("⏳ {message}...");
    }

    fn finish(&self) {}
}
