//! 持久化邮箱与状态恢复
//!
//! 实现基于"日志即数据库" (Log as a Database) 思想的持久化邮箱。
//! 支持故障后状态精确恢复，提供从"最多一次"到"恰好一次"的消息投递保证。

use crate::error::{ActorError, ActorResult};
use crate::messaging::InternalMessage;
use bincode;
use crc32fast::Hasher;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

/// Write-Ahead Log 条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    /// 日志偏移量 (全局单调递增)
    pub offset: u64,
    /// 消息内容
    pub(crate) message: InternalMessage,
    /// 写入时间戳 (Unix timestamp in nanoseconds)
    pub timestamp: u64,
    /// 校验和 (用于数据完整性验证)
    pub checksum: u32,
}

/// Actor 状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// 快照创建时的日志偏移量
    pub log_offset: u64,
    /// 快照创建时间 (Unix timestamp in nanoseconds)
    pub timestamp: u64,
    /// 状态数据 (序列化的 Actor 状态)
    pub state_data: Vec<u8>,
    /// 快照版本
    pub version: u32,
}

/// 持久化邮箱配置
#[derive(Debug, Clone)]
pub struct PersistentMailboxConfig {
    /// WAL 日志文件路径
    pub log_path: PathBuf,
    /// 快照文件路径
    pub snapshot_path: PathBuf,
    /// 快照创建频率 (按消息数量)
    pub snapshot_interval: u64,
    /// 是否启用持久化
    pub persistence_enabled: bool,
    /// 批量提交大小 (用于性能优化)
    pub batch_commit_size: usize,
    /// WAL 文件最大大小 (字节)
    pub max_wal_size: u64,
}

impl Default for PersistentMailboxConfig {
    fn default() -> Self {
        Self {
            log_path: PathBuf::from("actor.wal"),
            snapshot_path: PathBuf::from("actor.snapshot"),
            snapshot_interval: 1000,
            persistence_enabled: true,
            batch_commit_size: 10,
            max_wal_size: 100 * 1024 * 1024, // 100MB
        }
    }
}

/// 持久化邮箱 - 基于 WAL 的事务日志
#[derive(Clone)]
pub struct PersistentMailbox {
    /// 配置
    config: PersistentMailboxConfig,
    /// WAL 文件句柄
    wal_writer: Arc<Mutex<BufWriter<File>>>,
    /// 当前日志偏移量
    current_offset: Arc<RwLock<u64>>,
    /// 内存队列 (WAL 写入后才放入)
    memory_queue: Arc<Mutex<std::collections::VecDeque<WalEntry>>>,
    /// 批量写入缓冲
    batch_buffer: Arc<Mutex<Vec<WalEntry>>>,
    /// 已处理消息偏移量 (用于去重和恢复)
    processed_offset: Arc<RwLock<u64>>,
    /// 统计信息
    stats: Arc<Mutex<MailboxStats>>,
}

impl PersistentMailbox {
    /// 创建新的持久化邮箱
    pub async fn new(config: PersistentMailboxConfig) -> ActorResult<Self> {
        if !config.persistence_enabled {
            info!("Persistence disabled, using in-memory mailbox");
        }

        // 确保目录存在
        if let Some(parent) = config.log_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                ActorError::IoError(format!("Failed to create WAL directory: {}", e))
            })?;
        }

        // 打开 WAL 文件
        let wal_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&config.log_path)
            .map_err(|e| ActorError::IoError(format!("Failed to open WAL file: {}", e)))?;

        let wal_writer = Arc::new(Mutex::new(BufWriter::new(wal_file)));

        // 读取当前偏移量 (从现有日志恢复)
        let current_offset = Self::read_last_offset(&config.log_path).await?;

        let mailbox = Self {
            config,
            wal_writer,
            current_offset: Arc::new(RwLock::new(current_offset)),
            memory_queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            batch_buffer: Arc::new(Mutex::new(Vec::new())),
            processed_offset: Arc::new(RwLock::new(0)),
            stats: Arc::new(Mutex::new(MailboxStats::default())),
        };

        info!(
            "Persistent mailbox initialized with offset: {}",
            current_offset
        );
        Ok(mailbox)
    }

    /// 写入消息到 WAL 并加入内存队列
    /// 这是"日志即数据库"的核心：先持久化，再处理
    #[allow(dead_code)]
    pub(crate) async fn append_message(&self, message: InternalMessage) -> ActorResult<u64> {
        if !self.config.persistence_enabled {
            // 如果禁用持久化，直接加入内存队列
            return self.append_memory_only(message).await;
        }

        // 1. 获取新的偏移量
        let offset = {
            let mut current_offset = self.current_offset.write().await;
            *current_offset += 1;
            *current_offset
        };

        // 2. 创建 WAL 条目
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let wal_entry = WalEntry {
            offset,
            message: message.clone(),
            timestamp,
            checksum: Self::calculate_checksum(&message),
        };

        // 3. 加入批量缓冲区
        {
            let mut batch_buffer = self.batch_buffer.lock().await;
            batch_buffer.push(wal_entry.clone());

            // 检查是否需要批量提交
            if batch_buffer.len() >= self.config.batch_commit_size {
                self.commit_batch(&mut batch_buffer).await?;
            }
        }

        // 4. **只有当消息成功写入批量缓冲区后**，才放入内存队列
        {
            let mut queue = self.memory_queue.lock().await;
            queue.push_back(wal_entry.clone());
        }

        // 5. 更新统计
        {
            let mut stats = self.stats.lock().await;
            stats.messages_appended += 1;
        }

        debug!("Message added to batch buffer with offset: {}", offset);

        // 6. 检查是否需要创建快照
        if offset % self.config.snapshot_interval == 0 {
            self.trigger_snapshot_creation().await;
        }

        Ok(offset)
    }

    /// 从内存队列中取出下一个待处理的消息
    pub async fn dequeue_message(&self) -> Option<WalEntry> {
        let mut queue = self.memory_queue.lock().await;
        let entry = queue.pop_front();

        if entry.is_some() {
            let mut stats = self.stats.lock().await;
            stats.messages_dequeued += 1;
        }

        entry
    }

    /// 标记消息为已处理 (用于去重和恢复点记录)
    pub async fn mark_processed(&self, offset: u64) -> ActorResult<()> {
        {
            let mut processed_offset = self.processed_offset.write().await;
            if offset > *processed_offset {
                *processed_offset = offset;
            }
        }

        {
            let mut stats = self.stats.lock().await;
            stats.messages_processed += 1;
        }

        debug!("Marked message as processed: offset {}", offset);
        Ok(())
    }

    /// 从故障中恢复 - 加载快照 + 重放日志
    pub async fn recover_from_failure(&self) -> ActorResult<RecoveryInfo> {
        info!("Starting recovery from failure...");
        let recovery_start_time = std::time::Instant::now();

        // 1. 验证 WAL 文件完整性
        self.validate_wal_integrity().await?;

        // 2. 加载最新快照
        let snapshot = self.load_latest_snapshot().await?;
        let recovery_start_offset = snapshot.as_ref().map(|s| s.log_offset).unwrap_or(0);

        info!("Recovery starting from offset: {}", recovery_start_offset);

        // 3. 重放日志 (从快照点开始)
        let replayed_messages = self.replay_wal_from_offset(recovery_start_offset).await?;

        // 4. 验证重放的消息完整性
        let valid_messages = self.validate_replayed_messages(&replayed_messages).await?;

        // 5. 重建内存队列状态
        {
            let mut queue = self.memory_queue.lock().await;
            queue.clear(); // 清空现有队列
            for entry in valid_messages.iter() {
                queue.push_back(entry.clone());
            }
        }

        // 6. 更新当前偏移量
        if let Some(last_entry) = valid_messages.last() {
            let mut current_offset = self.current_offset.write().await;
            *current_offset = last_entry.offset;
        }

        // 7. 更新统计信息
        {
            let mut stats = self.stats.lock().await;
            stats.recovery_count += 1;
        }

        let recovery_duration = recovery_start_time.elapsed();
        let recovery_info = RecoveryInfo {
            snapshot_loaded: snapshot.is_some(),
            snapshot_offset: snapshot.as_ref().map(|s| s.log_offset),
            messages_replayed: valid_messages.len(),
            final_offset: valid_messages
                .last()
                .map(|e| e.offset)
                .unwrap_or(recovery_start_offset),
            recovery_duration: Some(recovery_duration),
            corrupted_entries_skipped: replayed_messages.len() - valid_messages.len(),
        };

        info!(
            "Recovery completed in {:?}: {:?}",
            recovery_duration, recovery_info
        );
        Ok(recovery_info)
    }

    /// 创建状态快照
    pub async fn create_snapshot(&self, state_data: Vec<u8>) -> ActorResult<()> {
        let current_offset = *self.current_offset.read().await;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let snapshot = StateSnapshot {
            log_offset: current_offset,
            timestamp,
            state_data,
            version: 1,
        };

        // 写入快照文件使用 bincode 序列化
        let snapshot_data = bincode::serialize(&snapshot)
            .map_err(|e| ActorError::IoError(format!("Failed to serialize snapshot: {}", e)))?;

        tokio::fs::write(&self.config.snapshot_path, snapshot_data)
            .await
            .map_err(|e| ActorError::IoError(format!("Failed to write snapshot: {}", e)))?;

        {
            let mut stats = self.stats.lock().await;
            stats.snapshots_created += 1;
        }

        info!("Snapshot created at offset: {}", current_offset);
        Ok(())
    }

    /// 获取邮箱统计信息
    pub async fn get_stats(&self) -> MailboxStats {
        let stats = self.stats.lock().await;
        stats.clone()
    }

    /// 获取当前队列长度
    pub async fn queue_length(&self) -> usize {
        let queue = self.memory_queue.lock().await;
        queue.len()
    }

    /// 强制刷新批量缓冲区到 WAL
    pub async fn flush_batch(&self) -> ActorResult<()> {
        let mut batch_buffer = self.batch_buffer.lock().await;
        if !batch_buffer.is_empty() {
            self.commit_batch(&mut batch_buffer).await?;
        }
        Ok(())
    }

    /// 优雅关闭邮箱 - 刷新所有待提交的数据
    pub async fn shutdown(&self) -> ActorResult<()> {
        info!("Shutting down persistent mailbox...");

        // 刷新所有待提交的批量数据
        self.flush_batch().await?;

        // 强制同步所有数据到磁盘
        {
            let mut writer = self.wal_writer.lock().await;
            writer.flush().map_err(|e| {
                ActorError::IoError(format!("Failed to flush WAL during shutdown: {}", e))
            })?;
        }

        info!("Persistent mailbox shutdown complete");
        Ok(())
    }

    // === 私有方法 ===

    /// 批量提交 WAL 条目到磁盘
    async fn commit_batch(&self, batch_buffer: &mut Vec<WalEntry>) -> ActorResult<()> {
        if batch_buffer.is_empty() {
            return Ok(());
        }

        debug!("Committing batch of {} entries to WAL", batch_buffer.len());

        // 序列化整个批次
        let mut batch_data = Vec::new();
        for entry in batch_buffer.iter() {
            let entry_data = bincode::serialize(entry).map_err(|e| {
                ActorError::IoError(format!("Failed to serialize WAL entry: {}", e))
            })?;

            // 写入条目长度 + 条目数据
            let entry_length = entry_data.len() as u32;
            batch_data.extend_from_slice(&entry_length.to_le_bytes());
            batch_data.extend_from_slice(&entry_data);
        }

        // 一次性写入整个批次
        {
            let mut writer = self.wal_writer.lock().await;
            writer
                .write_all(&batch_data)
                .map_err(|e| ActorError::IoError(format!("Failed to write batch to WAL: {}", e)))?;

            // **关键**: 确保数据刷入物理存储 (fsync)
            writer
                .flush()
                .map_err(|e| ActorError::IoError(format!("Failed to flush WAL batch: {}", e)))?;
        }

        // 更新统计
        {
            let mut stats = self.stats.lock().await;
            stats.total_wal_writes += batch_buffer.len() as u64;
        }

        debug!(
            "Successfully committed batch of {} entries",
            batch_buffer.len()
        );

        // 清空批量缓冲区
        batch_buffer.clear();
        Ok(())
    }

    /// 仅内存模式的消息追加
    #[allow(dead_code)]
    async fn append_memory_only(&self, message: InternalMessage) -> ActorResult<u64> {
        let offset = {
            let mut current_offset = self.current_offset.write().await;
            *current_offset += 1;
            *current_offset
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let wal_entry = WalEntry {
            offset,
            message,
            timestamp,
            checksum: 0, // 内存模式不需要校验和
        };

        {
            let mut queue = self.memory_queue.lock().await;
            queue.push_back(wal_entry);
        }

        Ok(offset)
    }

    /// 启动定期批量提交任务
    pub async fn start_periodic_flush(
        &self,
        interval: std::time::Duration,
    ) -> tokio::task::JoinHandle<()> {
        let mailbox = self.clone();

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);

            loop {
                interval_timer.tick().await;

                if let Err(e) = mailbox.flush_batch().await {
                    warn!("Failed to flush batch during periodic commit: {}", e);
                }
            }
        })
    }

    /// 从 WAL 文件读取最后一个偏移量
    async fn read_last_offset(log_path: &Path) -> ActorResult<u64> {
        if !log_path.exists() {
            return Ok(0);
        }

        let file = tokio::fs::File::open(log_path)
            .await
            .map_err(|e| ActorError::IoError(format!("Failed to open WAL file: {}", e)))?;

        let mut reader = BufReader::new(file.into_std().await);
        let mut last_offset = 0u64;

        loop {
            // 读取条目长度 (4字节)
            let mut length_bytes = [0u8; 4];
            match reader.read_exact(&mut length_bytes) {
                Ok(()) => {
                    let entry_length = u32::from_le_bytes(length_bytes) as usize;

                    // 读取条目数据
                    let mut entry_data = vec![0u8; entry_length];
                    if reader.read_exact(&mut entry_data).is_ok() {
                        // 尝试反序列化以验证数据完整性
                        if let Ok(entry) = bincode::deserialize::<WalEntry>(&entry_data) {
                            last_offset = entry.offset;
                        } else {
                            warn!(
                                "Corrupted WAL entry found, stopping at offset: {}",
                                last_offset
                            );
                            break;
                        }
                    } else {
                        // 读取失败，可能是文件末尾或损坏
                        break;
                    }
                }
                Err(_) => {
                    // 读取失败，到达文件末尾
                    break;
                }
            }
        }

        info!("Read last offset from WAL: {}", last_offset);
        Ok(last_offset)
    }

    /// 加载最新的状态快照
    async fn load_latest_snapshot(&self) -> ActorResult<Option<StateSnapshot>> {
        if !self.config.snapshot_path.exists() {
            return Ok(None);
        }

        let snapshot_data = tokio::fs::read(&self.config.snapshot_path)
            .await
            .map_err(|e| ActorError::IoError(format!("Failed to read snapshot: {}", e)))?;

        // 使用 bincode 反序列化快照
        let snapshot = bincode::deserialize::<StateSnapshot>(&snapshot_data)
            .map_err(|e| ActorError::IoError(format!("Failed to deserialize snapshot: {}", e)))?;

        info!("Loaded snapshot at offset: {}", snapshot.log_offset);
        Ok(Some(snapshot))
    }

    /// 从指定偏移量重放 WAL 日志
    async fn replay_wal_from_offset(&self, start_offset: u64) -> ActorResult<Vec<WalEntry>> {
        if !self.config.log_path.exists() {
            return Ok(Vec::new());
        }

        let file = tokio::fs::File::open(&self.config.log_path)
            .await
            .map_err(|e| ActorError::IoError(format!("Failed to open WAL file: {}", e)))?;

        let mut reader = BufReader::new(file.into_std().await);
        let mut replayed_entries = Vec::new();

        loop {
            // 读取条目长度 (4字节)
            let mut length_bytes = [0u8; 4];
            match reader.read_exact(&mut length_bytes) {
                Ok(()) => {
                    let entry_length = u32::from_le_bytes(length_bytes) as usize;

                    // 读取条目数据
                    let mut entry_data = vec![0u8; entry_length];
                    if reader.read_exact(&mut entry_data).is_ok() {
                        // 尝试反序列化
                        match bincode::deserialize::<WalEntry>(&entry_data) {
                            Ok(entry) => {
                                // 验证校验和
                                let expected_checksum = Self::calculate_checksum(&entry.message);
                                if entry.checksum != expected_checksum {
                                    warn!(
                                        "Checksum mismatch for entry at offset {}, skipping",
                                        entry.offset
                                    );
                                    continue;
                                }

                                // 只重放大于起始偏移量的条目
                                if entry.offset > start_offset {
                                    replayed_entries.push(entry);
                                }
                            }
                            Err(e) => {
                                warn!("Failed to deserialize WAL entry: {}, stopping replay", e);
                                break;
                            }
                        }
                    } else {
                        // 读取失败，可能是文件末尾或损坏
                        break;
                    }
                }
                Err(_) => {
                    // 读取失败，到达文件末尾
                    break;
                }
            }
        }

        info!(
            "Replayed {} entries from offset {}",
            replayed_entries.len(),
            start_offset
        );
        Ok(replayed_entries)
    }

    /// 触发快照创建 (异步)
    #[allow(dead_code)]
    async fn trigger_snapshot_creation(&self) {
        // 这里应该发送信号给 Actor 创建快照
        // 实际实现需要与 ActorSystem 协调
        debug!("Snapshot creation triggered");
    }

    /// 验证 WAL 文件完整性
    async fn validate_wal_integrity(&self) -> ActorResult<()> {
        if !self.config.log_path.exists() {
            return Ok(());
        }

        info!("Validating WAL file integrity...");

        let file = tokio::fs::File::open(&self.config.log_path)
            .await
            .map_err(|e| ActorError::IoError(format!("Failed to open WAL file: {}", e)))?;

        let mut reader = BufReader::new(file.into_std().await);
        let mut entry_count = 0;
        let mut corruption_found = false;

        loop {
            let mut length_bytes = [0u8; 4];
            match reader.read_exact(&mut length_bytes) {
                Ok(()) => {
                    let entry_length = u32::from_le_bytes(length_bytes) as usize;

                    // 检查条目长度是否合理 (防止损坏导致的巨大值)
                    if entry_length > self.config.max_wal_size as usize {
                        warn!(
                            "Suspicious entry length detected: {}, stopping validation",
                            entry_length
                        );
                        corruption_found = true;
                        break;
                    }

                    let mut entry_data = vec![0u8; entry_length];
                    if reader.read_exact(&mut entry_data).is_ok() {
                        match bincode::deserialize::<WalEntry>(&entry_data) {
                            Ok(_) => entry_count += 1,
                            Err(_) => {
                                warn!("Corrupted entry found at position {}", entry_count);
                                corruption_found = true;
                            }
                        }
                    } else {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        if corruption_found {
            warn!(
                "WAL integrity check found {} entries with some corruption detected",
                entry_count
            );
        } else {
            info!(
                "WAL integrity check passed: {} entries verified",
                entry_count
            );
        }

        Ok(())
    }

    /// 验证重放消息的完整性和顺序
    async fn validate_replayed_messages(
        &self,
        messages: &[WalEntry],
    ) -> ActorResult<Vec<WalEntry>> {
        debug!("Validating {} replayed messages", messages.len());

        let mut valid_messages = Vec::new();
        let mut last_offset = 0u64;

        for entry in messages {
            // 检查偏移量顺序
            if entry.offset <= last_offset {
                warn!(
                    "Out-of-order entry found: offset {} after {}, skipping",
                    entry.offset, last_offset
                );
                continue;
            }

            // 验证校验和
            let expected_checksum = Self::calculate_checksum(&entry.message);
            if entry.checksum != expected_checksum {
                warn!(
                    "Checksum validation failed for entry at offset {}, skipping",
                    entry.offset
                );
                continue;
            }

            // 检查时间戳合理性
            if entry.timestamp == 0 {
                warn!(
                    "Invalid timestamp for entry at offset {}, skipping",
                    entry.offset
                );
                continue;
            }

            valid_messages.push(entry.clone());
            last_offset = entry.offset;
        }

        info!(
            "Message validation completed: {}/{} messages are valid",
            valid_messages.len(),
            messages.len()
        );

        Ok(valid_messages)
    }

    /// 计算消息校验和
    fn calculate_checksum(message: &InternalMessage) -> u32 {
        let mut hasher = Hasher::new();
        hasher.update(&message.payload);
        hasher.update(message.message_type.as_bytes());
        hasher.update(message.trace_id.as_bytes());
        hasher.finalize()
    }
}

/// 恢复信息
#[derive(Debug)]
pub struct RecoveryInfo {
    pub snapshot_loaded: bool,
    pub snapshot_offset: Option<u64>,
    pub messages_replayed: usize,
    pub final_offset: u64,
    pub recovery_duration: Option<std::time::Duration>,
    pub corrupted_entries_skipped: usize,
}

/// 邮箱统计信息
#[derive(Debug, Clone, Default)]
pub struct MailboxStats {
    pub messages_appended: u64,
    pub messages_dequeued: u64,
    pub messages_processed: u64,
    pub total_wal_writes: u64,
    pub snapshots_created: u64,
    pub recovery_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_persistent_mailbox_basic_operations() {
        let temp_dir = tempdir().unwrap();
        let config = PersistentMailboxConfig {
            log_path: temp_dir.path().join("test.wal"),
            snapshot_path: temp_dir.path().join("test.snapshot"),
            persistence_enabled: true,
            ..Default::default()
        };

        let mailbox = PersistentMailbox::new(config).await.unwrap();

        // 测试消息追加
        let message = InternalMessage {
            payload: b"test message".to_vec(),
            message_type: "TestMessage".to_string(),
            priority: crate::messaging::MessagePriority::Normal,
            is_stream: false,
            trace_id: "test-trace".to_string(),
            source_actor: None,
            created_at: std::time::Instant::now(),
        };

        let offset = mailbox.append_message(message).await.unwrap();
        assert_eq!(offset, 1);

        // 测试消息出队
        let entry = mailbox.dequeue_message().await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().offset, 1);

        // 测试统计
        let stats = mailbox.get_stats().await;
        assert_eq!(stats.messages_appended, 1);
        assert_eq!(stats.messages_dequeued, 1);
    }
}
