#!/bin/bash
# Actrix 数据备份脚本

set -e

BACKUP_DIR="${BACKUP_DIR:-./backup}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_PATH="$BACKUP_DIR/$TIMESTAMP"

echo "🔄 开始备份 Actrix 数据..."
echo "备份目录: $BACKUP_PATH"
echo

# 创建备份目录
mkdir -p "$BACKUP_PATH"

# 备份配置文件
echo "1. 备份配置文件..."
if [ -f "config.toml" ]; then
    cp config.toml "$BACKUP_PATH/config.toml"
    echo "  ✅ config.toml"
fi

# 备份数据库文件
echo
echo "2. 备份数据库..."
find . -name "*.db" -type f | while read -r db; do
    db_name=$(basename "$db")
    # 使用 SQLite VACUUM INTO 创建干净的备份副本
    if command -v sqlite3 &> /dev/null; then
        sqlite3 "$db" "VACUUM INTO '$BACKUP_PATH/$db_name'"
        echo "  ✅ $db_name (VACUUM)"
    else
        cp "$db" "$BACKUP_PATH/$db_name"
        echo "  ✅ $db_name (copy)"
    fi
done

# 备份日志（最近7天）
echo
echo "3. 备份最近日志..."
if [ -d "logs" ]; then
    mkdir -p "$BACKUP_PATH/logs"
    find logs -name "*.log" -mtime -7 -exec cp {} "$BACKUP_PATH/logs/" \;
    echo "  ✅ 最近7天日志"
fi

# 压缩备份
echo
echo "4. 压缩备份..."
cd "$BACKUP_DIR"
tar -czf "${TIMESTAMP}.tar.gz" "$TIMESTAMP"
rm -rf "$TIMESTAMP"
echo "  ✅ ${TIMESTAMP}.tar.gz"

# 清理旧备份（保留最近30天）
echo
echo "5. 清理旧备份..."
find . -name "*.tar.gz" -mtime +30 -delete
count=$(find . -name "*.tar.gz" | wc -l)
echo "  ✅ 保留 $count 个备份文件"

echo
echo "✅ 备份完成: $BACKUP_DIR/${TIMESTAMP}.tar.gz"
echo "恢复命令: tar -xzf $BACKUP_DIR/${TIMESTAMP}.tar.gz"
