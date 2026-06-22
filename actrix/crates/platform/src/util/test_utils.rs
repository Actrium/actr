//! 测试工具模块
//!
//! 提供测试相关的工具函数和辅助功能

#[cfg(test)]
pub mod utils {
    use std::path::Path;
    use tokio::sync::OnceCell;

    // Initialize the test database once globally
    static INIT: OnceCell<()> = OnceCell::const_new();

    // Setup test database
    pub async fn setup_test_db() -> anyhow::Result<()> {
        INIT.get_or_init(|| async {
            // Use a directory for the database, not a file directly
            let db_dir = std::env::temp_dir().join("actrix_test_db");

            // Ensure directory exists
            std::fs::create_dir_all(&db_dir).expect("Failed to create test database directory");

            // Remove existing database file if it exists
            let db_file = db_dir.join("actrix.db");
            if db_file.exists() {
                let _ = std::fs::remove_file(&db_file);
            }

            let db_dir_str = db_dir
                .to_str()
                .expect("Failed to convert DB directory path to string");

            // Try to set database path, ignore error if already initialized
            match crate::storage::db::set_db_path(Path::new(db_dir_str)).await {
                Ok(()) => {}
                Err(e) => {
                    // If database is already initialized, that's okay for tests
                    let err_msg = e.to_string();
                    if !err_msg.contains("already initialized")
                        && !err_msg.contains("Database already")
                    {
                        panic!("Failed to initialize test database: {e}");
                    }
                }
            }
        })
        .await;
        Ok(())
    }
}
