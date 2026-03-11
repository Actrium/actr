//! Exponential Backoff retry strategy
//!
//! Used for retry delay calculation after connection failures

use std::time::Duration;

/// Exponential backoff strategy
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    /// Current retry attempt count
    attempt: u32,

    /// Initial delay in milliseconds
    initial_delay_ms: u64,

    /// Maximum delay in milliseconds
    max_delay_ms: u64,

    /// Multiplier factor
    multiplier: f64,

    /// Random jitter factor (0.0 - 1.0)
    jitter: f64,
}

impl ExponentialBackoff {
    /// Create a new exponential backoff strategy
    ///
    /// # Parameters
    /// - `initial_delay_ms`: initial delay in milliseconds
    /// - `max_delay_ms`: maximum delay in milliseconds
    pub fn new(initial_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            attempt: 0,
            initial_delay_ms,
            max_delay_ms,
            multiplier: 2.0,
            jitter: 0.1,
        }
    }

    /// Set the multiplier factor
    pub fn with_multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = multiplier;
        self
    }

    /// Set the jitter factor
    pub fn with_jitter(mut self, jitter: f64) -> Self {
        self.jitter = jitter.clamp(0.0, 1.0);
        self
    }

    /// Get the delay for the next retry
    pub fn next_delay(&mut self) -> Duration {
        let base_delay = (self.initial_delay_ms as f64 * self.multiplier.powi(self.attempt as i32))
            .min(self.max_delay_ms as f64);

        // Add simple jitter (based on attempt)
        let jitter_range = base_delay * self.jitter;
        // Use attempt as a pseudo-random source
        let pseudo_random = ((self.attempt * 7919) % 100) as f64 / 100.0; // 0.0 - 1.0
        let jitter = (pseudo_random * 2.0 - 1.0) * jitter_range;
        let final_delay = (base_delay + jitter).max(0.0);

        self.attempt += 1;

        Duration::from_millis(final_delay as u64)
    }

    /// Reset the retry counter
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Get the current retry attempt count
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new(1000, 30000) // 1s - 30s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff() {
        let mut backoff = ExponentialBackoff::new(1000, 10000);

        let delay1 = backoff.next_delay();
        assert!(delay1.as_millis() >= 900 && delay1.as_millis() <= 1100);

        let delay2 = backoff.next_delay();
        assert!(delay2.as_millis() >= 1800 && delay2.as_millis() <= 2200);

        let delay3 = backoff.next_delay();
        assert!(delay3.as_millis() >= 3600 && delay3.as_millis() <= 4400);

        // Reset
        backoff.reset();
        let delay4 = backoff.next_delay();
        assert!(delay4.as_millis() >= 900 && delay4.as_millis() <= 1100);
    }

    #[test]
    fn test_max_delay() {
        let mut backoff = ExponentialBackoff::new(1000, 5000);

        for _ in 0..10 {
            backoff.next_delay();
        }

        let delay = backoff.next_delay();
        assert!(delay.as_millis() <= 5500); // accounting for jitter
    }
}
