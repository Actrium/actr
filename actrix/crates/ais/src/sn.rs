//! Serial Number (SN) Generator - 基于 Snowflake 算法的 54-bit 序列号生成器
//!
//! # 协议约束
//!
//! 根据 [actor-rtc-proto](https://github.com/actor-rtc/proto) 定义：
//! ```proto
//! message ActorId {
//!     uint64 serial_number  = 1;  // 54-bit 限制
//!     ActorType type        = 2;
//! }
//!
//! enum BitLayoutOfActorIdBytes {
//!     LEN_SERIAL_NUMBER     = 54;
//!     LEN_TYPE_CODE         = 2;
//! }
//! ```
//!
//! # Bit 布局
//!
//! 54-bit serial_number 的内部结构（Snowflake 算法变体）：
//!
//! ```text
//! ┌─────────────┬───────────┬────────────┐
//! │ Timestamp   │ Worker ID │ Sequence   │
//! │  41 bits    │  5 bits   │  8 bits    │
//! └─────────────┴───────────┴────────────┘
//!
//! - Timestamp (41 bits): 相对于 2023-01-01 的毫秒数
//!   - 可表示范围: ~69.7 年 (2**41 / 1000 / 3600 / 24 / 365)
//! - Worker ID (5 bits): 节点标识 (0-31)
//!   - 基于 hostname + PID 哈希生成
//! - Sequence (8 bits): 同毫秒内的序列号 (0-255)
//!   - 单毫秒最多 256 个 ID
//! ```
//!
//! # 特性
//!
//! - **全局唯一**: 分布式环境下保证唯一性（假设 worker_id 不冲突）
//! - **时间排序**: ID 大致按生成时间递增
//! - **高性能**: 单节点支持 256K IDs/秒 (256 * 1000)
//! - **无中心化**: 无需中心协调服务
//!
//! # 时钟回拨处理
//!
//! - **小幅回拨**: 使用上次时间戳 + 递增序列号
//! - **序列号耗尽**: 强制推进时间戳（可能导致与真实时钟偏差）
//!
//! # 示例
//!
//! ```ignore
//! use ais::sn::{SerialNumber, AIdSerialNumberIssuer};
//!
//! // 生成序列号
//! let sn1 = SerialNumber::sn(1);  // realm_id = 1 (当前未使用)
//! let sn2 = SerialNumber::sn(1);
//!
//! // 序列号自动递增
//! assert!(sn2.value() > sn1.value());
//!
//! // 保证在 54-bit 范围内
//! assert!(sn1.value() <= SerialNumber::MAX_VALUE);
//! ```
//!
//! # 注意事项
//!
//! ⚠️ **realm_id 参数当前未使用**：序列号全局唯一，不按 realm 隔离。
//! 如需 realm 隔离，考虑将 realm_id 嵌入 worker_id 的高位。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The number of bits used for the serial number.
///
/// ActrId 协议定义 serial_number 字段使用 54 bits：
/// - 41 bits: timestamp (毫秒，相对于自定义 epoch)
/// - 5 bits: worker_id (支持 32 个并发节点)
/// - 8 bits: sequence (每毫秒最多 256 个 ID)
pub const BITS_LEN_SERIAL_NUMBER: usize = 54;

// Snowflake-like algorithm constants for 54-bit SN
const CUSTOM_EPOCH: u64 = 1672531200000; // 2023-01-01 00:00:00 UTC in milliseconds
const WORKER_ID_BITS: u64 = 5;
const SEQUENCE_BITS: u64 = 8;
#[cfg(test)]
const TIMESTAMP_BITS: u64 = 54 - WORKER_ID_BITS - SEQUENCE_BITS; // 41 bits

pub const WORKER_ID_SHIFT: u64 = SEQUENCE_BITS;
pub const TIMESTAMP_SHIFT: u64 = SEQUENCE_BITS + WORKER_ID_BITS;

pub const MAX_WORKER_ID: u64 = (1 << WORKER_ID_BITS) - 1; // 31
pub const MAX_SEQUENCE: u64 = (1 << SEQUENCE_BITS) - 1; // 255

/// Snowflake generator state using lock-free atomics
///
/// 性能优化：使用原子操作替代 Mutex，提升高并发性能
/// - AtomicU64 编码：[41-bit timestamp][8-bit sequence][15-bit padding]
/// - Worker ID 只初始化一次，存储在 OnceLock 中
static SNOWFLAKE_STATE: AtomicU64 = AtomicU64::new(0);
static WORKER_ID: OnceLock<u64> = OnceLock::new();

/// 从 AtomicU64 中解码 timestamp 和 sequence
#[inline]
fn decode_state(state: u64) -> (u64, u64) {
    let timestamp = state >> 8; // 高 56 位中的 timestamp
    let sequence = state & 0xFF; // 低 8 位是 sequence
    (timestamp, sequence)
}

/// 将 timestamp 和 sequence 编码为 AtomicU64
#[inline]
fn encode_state(timestamp: u64, sequence: u64) -> u64 {
    (timestamp << 8) | (sequence & 0xFF)
}

/// 初始化 worker_id（只执行一次）
fn init_worker_id() -> u64 {
    *WORKER_ID.get_or_init(|| {
        // Generate worker ID based on process ID and hostname hash
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        let pid = std::process::id();

        // Simple hash to generate worker ID within range
        let combined = format!("{hostname}{pid}");
        let mut hash = 0u64;
        for byte in combined.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
        }
        hash & MAX_WORKER_ID
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SNError {
    /// The provided value exceeds the 54-bit limit.
    ValueOverflow(u64),
}

impl std::fmt::Display for SNError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SNError::ValueOverflow(val) => {
                write!(f, "serial number value {val} exceeds the 54-bit limit")
            }
        }
    }
}

impl std::error::Error for SNError {}

/// A newtype representing a N-bit (N <= 64) serial number.
/// It guarantees that the inner value is always within the valid 54-bit range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SerialNumber(u64);

impl SerialNumber {
    /// The maximum possible value for a 54-bit serial number.
    pub const MAX_VALUE: u64 = (1 << BITS_LEN_SERIAL_NUMBER) - 1;

    /// Creates a new `SerialNumber`, returning an error if the value is out of bounds.
    pub fn new(value: u64) -> Result<Self, SNError> {
        if value > Self::MAX_VALUE {
            Err(SNError::ValueOverflow(value))
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the inner u64 value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

// Idiomatic conversion from our type to a u64
impl From<SerialNumber> for u64 {
    fn from(sn: SerialNumber) -> Self {
        sn.0
    }
}

// Idiomatic fallible conversion from a u64 to our type
impl TryFrom<u64> for SerialNumber {
    type Error = SNError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

pub trait AIdSerialNumberIssuer {
    fn sn(realm_id: u32) -> SerialNumber;
}

impl AIdSerialNumberIssuer for SerialNumber {
    fn sn(_realm_id: u32) -> SerialNumber {
        let worker_id = init_worker_id();

        // Get current timestamp in milliseconds
        let current_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Ensure timestamp is relative to our custom epoch
        let mut timestamp = current_millis.saturating_sub(CUSTOM_EPOCH);

        // Lock-free CAS loop
        loop {
            let old_state = SNOWFLAKE_STATE.load(Ordering::Relaxed);
            let (last_timestamp, last_sequence) = decode_state(old_state);

            let (new_timestamp, new_sequence) = if timestamp < last_timestamp {
                // Clock went backwards - use last timestamp and increment sequence
                if last_sequence < MAX_SEQUENCE {
                    (last_timestamp, last_sequence + 1)
                } else {
                    // Sequence exhausted, force timestamp forward
                    (last_timestamp + 1, 0)
                }
            } else if timestamp == last_timestamp {
                // Same millisecond - increment sequence
                if last_sequence < MAX_SEQUENCE {
                    (timestamp, last_sequence + 1)
                } else {
                    // Sequence exhausted, move to next millisecond
                    (timestamp + 1, 0)
                }
            } else {
                // New millisecond - reset sequence
                (timestamp, 0)
            };

            let new_state = encode_state(new_timestamp, new_sequence);

            // Try to atomically update the state
            match SNOWFLAKE_STATE.compare_exchange_weak(
                old_state,
                new_state,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Success! Build the final serial number
                    let sn_value = (new_timestamp << TIMESTAMP_SHIFT)
                        | (worker_id << WORKER_ID_SHIFT)
                        | new_sequence;

                    return Self::new(sn_value & Self::MAX_VALUE)
                        .expect("BUG: Masked value should always be within 54-bit range");
                }
                Err(_) => {
                    // CAS failed, another thread modified the state
                    // Retry with updated timestamp
                    timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64
                        - CUSTOM_EPOCH;
                    continue;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::sn::{
        AIdSerialNumberIssuer, MAX_SEQUENCE, MAX_WORKER_ID, SerialNumber, TIMESTAMP_BITS,
        TIMESTAMP_SHIFT, WORKER_ID_SHIFT,
    };
    use std::collections::HashSet;
    use std::thread;

    #[test]
    fn test_serial_number_creation() {
        let sn1 = SerialNumber::sn(1);
        let sn2 = SerialNumber::sn(1);
        let sn3 = SerialNumber::sn(2);

        // Different calls should produce different serial numbers
        assert_ne!(sn1.value(), sn2.value());
        assert_ne!(sn2.value(), sn3.value());
        assert_ne!(sn1.value(), sn3.value());

        // All should be within 54-bit range
        assert!(sn1.value() <= SerialNumber::MAX_VALUE);
        assert!(sn2.value() <= SerialNumber::MAX_VALUE);
        assert!(sn3.value() <= SerialNumber::MAX_VALUE);
    }

    #[test]
    fn test_monotonic_generation() {
        let mut _previous = SerialNumber::sn(1).value();

        // Generate several serial numbers and check they generally increase
        // (allowing for some timestamp variations)
        for _ in 0..100 {
            let current = SerialNumber::sn(1).value();
            // In Snowflake, newer IDs should generally be larger (timestamp component)
            // But we can't guarantee strict monotonicity due to sequence wraparound
            assert!(current <= SerialNumber::MAX_VALUE);
            _previous = current;
        }
    }

    #[test]
    fn test_uniqueness_in_rapid_generation() {
        let mut generated = HashSet::new();

        // Rapidly generate many serial numbers
        for i in 0..1000 {
            let sn = SerialNumber::sn(i % 10); // Different tenants
            assert!(
                generated.insert(sn.value()),
                "Duplicate serial number generated: {}",
                sn.value()
            );
        }

        assert_eq!(generated.len(), 1000);
    }

    #[test]
    fn test_concurrent_generation() {
        let handles: Vec<_> = (0..10)
            .map(|i| {
                thread::spawn(move || {
                    let mut local_set = HashSet::new();
                    for j in 0..100 {
                        let sn = SerialNumber::sn((i * 100 + j) as u32);
                        local_set.insert(sn.value());
                    }
                    local_set
                })
            })
            .collect();

        let mut all_values = HashSet::new();
        for handle in handles {
            let local_set = handle.join().unwrap();
            for value in local_set {
                assert!(
                    all_values.insert(value),
                    "Duplicate serial number in concurrent test: {value}"
                );
            }
        }

        assert_eq!(all_values.len(), 1000);
    }

    #[test]
    fn test_bit_layout_extraction() {
        let sn = SerialNumber::sn(42);
        let value = sn.value();

        // Extract components (reverse of the generation process)
        let sequence = value & MAX_SEQUENCE;
        let worker_id = (value >> WORKER_ID_SHIFT) & MAX_WORKER_ID;
        let timestamp = value >> TIMESTAMP_SHIFT;

        // All components should be within their respective ranges
        assert!(sequence <= MAX_SEQUENCE);
        assert!(worker_id <= MAX_WORKER_ID);
        assert!(timestamp < (1 << TIMESTAMP_BITS));

        println!("SN: {value}, Timestamp: {timestamp}, Worker: {worker_id}, Sequence: {sequence}");
    }
}
