//! Input handling utilities

use anyhow::Result;
use dialoguer::{Select, theme::ColorfulTheme};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Enhanced select with Ctrl+C interrupt support
pub struct EnhancedSelect<'a> {
    prompt: String,
    items: Vec<String>,
    theme: &'a ColorfulTheme,
    allow_interrupt: bool,
    interrupted: Option<Arc<AtomicBool>>,
}

/// Result of enhanced select
pub enum SelectResult {
    Selected(usize),
    Interrupted,
}

impl<'a> EnhancedSelect<'a> {
    pub fn new(theme: &'a ColorfulTheme) -> Self {
        Self {
            prompt: String::new(),
            items: Vec::new(),
            theme,
            allow_interrupt: false,
            interrupted: None,
        }
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    pub fn items(mut self, items: &[impl ToString]) -> Self {
        self.items = items.iter().map(|item| item.to_string()).collect();
        self
    }

    pub fn with_interrupt(mut self, allow: bool, interrupted: Option<Arc<AtomicBool>>) -> Self {
        self.allow_interrupt = allow;
        self.interrupted = interrupted;
        self
    }

    pub fn interact(self) -> Result<SelectResult> {
        // Check if Ctrl+C was pressed before showing the menu
        if let Some(ref interrupted) = self.interrupted {
            if interrupted.load(Ordering::SeqCst) {
                return Ok(SelectResult::Interrupted);
            }
        }

        // Use dialoguer's select but catch interruption
        match Select::with_theme(self.theme)
            .with_prompt(&self.prompt)
            .items(&self.items)
            .default(0)
            .interact_opt()
        {
            Ok(Some(selection)) => Ok(SelectResult::Selected(selection)),
            Ok(None) => {
                // This happens when user presses Ctrl+C
                Ok(SelectResult::Interrupted)
            }
            Err(_) => {
                // All errors are treated as interruptions now
                Ok(SelectResult::Interrupted)
            }
        }
    }
}
