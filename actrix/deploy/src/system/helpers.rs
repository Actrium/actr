//! Helper functions for system operations

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Display keyboard help for confirmation dialogs
#[allow(unused)]
pub fn show_confirm_help() {
    println!("----------------------");
    println!("use y/n keys or enter for default, ctrl+c to exit");
    println!();
}

/// Clear input buffer to prevent fast keypress from affecting next input
pub fn clear_input_buffer() {
    // Try to drain any pending input from stdin
    // Set stdin to non-blocking mode temporarily
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;

        let stdin_fd = io::stdin().as_raw_fd();

        // Get current flags
        let flags = unsafe { libc::fcntl(stdin_fd, libc::F_GETFL) };
        if flags != -1 {
            // Set non-blocking
            unsafe { libc::fcntl(stdin_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };

            // Read and discard any pending bytes
            let mut buffer = [0u8; 1024];
            let mut stdin = io::stdin();
            while let Ok(n) = stdin.read(&mut buffer) {
                // Exit when no more data (non-blocking returns 0 or Err)
                if n == 0 {
                    break;
                }
            }

            // Restore original flags
            unsafe { libc::fcntl(stdin_fd, libc::F_SETFL, flags) };
        }
    }

    #[cfg(not(unix))]
    {
        // For non-Unix systems, just add a small delay
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Wait for any key press to continue, with interrupt support
pub fn press_any_key_to_with_interrupt(to: &str, interrupted: Arc<AtomicBool>) -> bool {
    // Check if already interrupted
    if interrupted.load(Ordering::SeqCst) {
        return true;
    }

    // Clear any buffered input first
    clear_input_buffer();

    // Use dialoguer's Input to handle key presses properly
    // This integrates better with terminal handling
    use dialoguer::{Input, theme::ColorfulTheme};

    print!("\nPress Enter to {}...", to);
    io::stdout().flush().unwrap();

    match Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("")
        .allow_empty(true)
        .interact()
    {
        Ok(_) => {
            // Got input (including empty), check if we were interrupted
            interrupted.load(Ordering::SeqCst)
        }
        Err(e) => {
            // Error usually means interrupt
            if format!("{}", e).contains("read interrupted") {
                true
            } else {
                // Other error, assume interrupted
                true
            }
        }
    }
}
