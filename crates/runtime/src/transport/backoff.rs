//! Exponential backoff retry strategy
//!
//! Provides configurable exponential backoff for connection retries and other
//! network operations that may fail transiently.

use std::time::Duration;

/// Exponential backoff iterator
///
/// Generates increasing delays using exponential backoff algorithm with configurable
/// initial delay, maximum delay, and retry limit.
///
/// # Example
/// ```
/// use std::time::Duration;
/// use actr_runtime::ExponentialBackoff;
///
/// let backoff = ExponentialBackoff::new(
///     Duration::from_millis(100),
///     Duration::from_secs(30),
///     Some(10),
/// );
///
/// for (attempt, delay) in backoff.enumerate() {
///     println!("Attempt {}: waiting {:?}", attempt, delay);
///     // Retry logic here
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    /// Current delay duration
    current: Duration,
    /// Maximum delay duration (cap)
    max: Duration,
    /// Multiplier for exponential growth (default: 2.0)
    multiplier: f64,
    /// Current retry count
    retries: u32,
    /// Maximum number of retries (None = unlimited)
    max_retries: Option<u32>,
}

impl ExponentialBackoff {
    /// Create new exponential backoff iterator
    ///
    /// # Arguments
    /// - `initial`: Initial delay duration
    /// - `max`: Maximum delay duration (delays will not exceed this)
    /// - `max_retries`: Maximum number of retries (None for unlimited)
    ///
    /// # Example
    /// ```
    /// use std::time::Duration;
    /// use actr_runtime::ExponentialBackoff;
    ///
    /// // Retry up to 5 times with delays: 100ms, 200ms, 400ms, 800ms, 1600ms
    /// let backoff = ExponentialBackoff::new(
    ///     Duration::from_millis(100),
    ///     Duration::from_secs(2),
    ///     Some(5),
    /// );
    /// ```
    pub fn new(initial: Duration, max: Duration, max_retries: Option<u32>) -> Self {
        Self {
            current: initial,
            max,
            multiplier: 2.0,
            retries: 0,
            max_retries,
        }
    }

    /// Create backoff with custom multiplier
    ///
    /// # Arguments
    /// - `initial`: Initial delay duration
    /// - `max`: Maximum delay duration
    /// - `max_retries`: Maximum number of retries
    /// - `multiplier`: Growth multiplier (e.g., 1.5 for slower growth)
    pub fn with_multiplier(
        initial: Duration,
        max: Duration,
        max_retries: Option<u32>,
        multiplier: f64,
    ) -> Self {
        Self {
            current: initial,
            max,
            multiplier,
            retries: 0,
            max_retries,
        }
    }

    /// Get current retry count
    pub fn retry_count(&self) -> u32 {
        self.retries
    }

    /// Reset backoff to initial state
    pub fn reset(&mut self) {
        self.retries = 0;
        self.current = Duration::from_millis(100); // Default initial
    }
}

impl Iterator for ExponentialBackoff {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        // Check if max retries reached
        if let Some(max_retries) = self.max_retries {
            if self.retries >= max_retries {
                return None;
            }
        }

        // Get current delay
        let delay = self.current;

        // Calculate next delay (exponential growth)
        let next_millis = (self.current.as_millis() as f64 * self.multiplier) as u64;
        let next_duration = Duration::from_millis(next_millis);

        // Cap at maximum delay
        self.current = if next_duration > self.max {
            self.max
        } else {
            next_duration
        };

        self.retries += 1;

        Some(delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff_basic() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(2), Some(4));

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(200)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(400)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(800)));
        assert_eq!(backoff.next(), None); // Exceeded max_retries
    }

    #[test]
    fn test_exponential_backoff_with_cap() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_millis(100),
            Duration::from_millis(500),
            Some(5),
        );

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(200)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(400)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(500))); // Capped
        assert_eq!(backoff.next(), Some(Duration::from_millis(500))); // Still capped
        assert_eq!(backoff.next(), None);
    }

    #[test]
    fn test_exponential_backoff_unlimited() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_millis(50),
            Duration::from_secs(10),
            None, // Unlimited retries
        );

        for i in 0..20 {
            let delay = backoff.next();
            assert!(delay.is_some(), "Retry {i} should succeed");
        }
    }

    #[test]
    fn test_custom_multiplier() {
        let mut backoff = ExponentialBackoff::with_multiplier(
            Duration::from_millis(100),
            Duration::from_secs(10),
            Some(3),
            1.5, // Slower growth
        );

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(150)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(225)));
        assert_eq!(backoff.next(), None);
    }

    #[test]
    fn test_retry_count() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(1), None);

        assert_eq!(backoff.retry_count(), 0);
        backoff.next();
        assert_eq!(backoff.retry_count(), 1);
        backoff.next();
        assert_eq!(backoff.retry_count(), 2);
    }
}
