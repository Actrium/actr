#!/bin/bash
# Actrix 安全检查脚本

set -e

echo "🔍 Actrix 安全检查..."
echo

# 检查数据库文件权限
echo "1. 检查数据库文件权限..."
find . -name "*.db" -type f | while read -r db; do
    perm=$(stat -c "%a" "$db" 2>/dev/null || stat -f "%A" "$db" 2>/dev/null)
    if [ "$perm" != "600" ]; then
        echo "  ⚠️  $db 权限为 $perm，应该是 600"
        echo "     修复: chmod 600 $db"
    else
        echo "  ✅ $db"
    fi
done

# 检查配置文件中的默认密钥
echo
echo "2. 检查默认密钥..."
if grep -r "default-.*-key\|change-in-production" config*.toml 2>/dev/null; then
    echo "  ⚠️  发现默认密钥，请修改！"
else
    echo "  ✅ 未发现默认密钥"
fi

# 检查密钥长度
echo
echo "3. 检查 actrix_shared_key 长度..."
if [ -f "config.toml" ]; then
    key=$(grep "actrix_shared_key" config.toml | cut -d'"' -f2)
    if [ ${#key} -lt 16 ]; then
        echo "  ⚠️  密钥长度 ${#key}，建议至少 16 字符"
    else
        echo "  ✅ 密钥长度 ${#key}"
    fi
fi

# 检查 TLS 证书
echo
echo "4. 检查 TLS 证书..."
if [ -f "config.toml" ]; then
    cert=$(grep "cert =" config.toml | cut -d'"' -f2 | head -1)
    if [ -n "$cert" ] && [ -f "$cert" ]; then
        expiry=$(openssl x509 -enddate -noout -in "$cert" 2>/dev/null | cut -d= -f2)
        echo "  ✅ 证书存在: $cert"
        echo "     过期时间: $expiry"
    else
        echo "  ⚠️  未配置或证书不存在"
    fi
fi

# 检查依赖漏洞
echo
echo "5. 检查依赖漏洞..."
if command -v cargo-audit &> /dev/null; then
    cargo audit
else
    echo "  ⚠️  cargo-audit 未安装，跳过"
    echo "     安装: cargo install cargo-audit"
fi

echo
echo "✅ 安全检查完成"
