#!/usr/bin/env bash
set -euo pipefail

# actr Workspace Release Script
# 用于自动化发布 actr workspace 的所有 crates 到 crates.io

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# 日志函数
log_info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*"
}

log_step() {
    echo -e "${CYAN}[STEP]${NC} $*"
}

# 显示使用方法
usage() {
    cat <<EOF
用法: $0 <版本类型> [选项]

版本类型:
  patch    递增补丁版本 (0.1.0 -> 0.1.1)
  minor    递增次版本 (0.1.0 -> 0.2.0)
  major    递增主版本 (0.1.0 -> 1.0.0)
  <版本号> 直接指定版本号 (如 1.2.3)

选项:
  --dry-run    只执行验证，不实际发布
  --no-verify  跳过测试（不推荐）
  --help       显示此帮助信息

示例:
  $0 patch                 # 发布补丁版本
  $0 minor --dry-run       # 测试次版本发布流程
  $0 1.0.0                 # 直接发布 1.0.0 版本
EOF
    exit 1
}

# Crates 发布顺序（按依赖关系排序）
CRATE_PUBLISH_ORDER=(
    "crates/protocol"
    "crates/version"
    "crates/framework"
    "crates/runtime-mailbox"
    "crates/config"
    "crates/framework-protoc-codegen"
    "crates/runtime"
    "."  # 主 crate
)

# 解析参数
VERSION_TYPE=""
DRY_RUN=false
NO_VERIFY=false

while [[ $# -gt 0 ]]; do
    case $1 in
        patch|minor|major)
            VERSION_TYPE="$1"
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --no-verify)
            NO_VERIFY=true
            shift
            ;;
        --help)
            usage
            ;;
        [0-9]*.[0-9]*.[0-9]*)
            VERSION_TYPE="$1"
            shift
            ;;
        *)
            log_error "未知参数: $1"
            usage
            ;;
    esac
done

if [[ -z "$VERSION_TYPE" ]]; then
    log_error "请指定版本类型"
    usage
fi

# 获取当前版本
get_current_version() {
    grep '^\[workspace\.package\]' -A 20 Cargo.toml | grep '^version = ' | head -n1 | sed -E 's/version = "(.*)"/\1/'
}

# 递增版本号
increment_version() {
    local version=$1
    local type=$2

    IFS='.' read -r major minor patch <<< "$version"

    case $type in
        major)
            echo "$((major + 1)).0.0"
            ;;
        minor)
            echo "${major}.$((minor + 1)).0"
            ;;
        patch)
            echo "${major}.${minor}.$((patch + 1))"
            ;;
        *)
            echo "$type"
            ;;
    esac
}

# 检查 Git 状态
check_git_status() {
    log_info "检查 Git 状态..."

    if [[ -n $(git status --porcelain) ]]; then
        log_error "工作目录不干净，请先提交或暂存更改"
        git status --short
        exit 1
    fi

    local branch=$(git rev-parse --abbrev-ref HEAD)
    if [[ "$branch" != "overhaul" && "$branch" != "main" ]]; then
        log_warn "当前分支: $branch (建议使用 overhaul 或 main)"
        read -p "是否继续? [y/N] " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 1
        fi
    fi

    log_success "Git 状态检查通过"
}

# 更新版本号
update_version() {
    local new_version=$1

    log_info "更新 workspace 版本号: $CURRENT_VERSION -> $new_version"

    # 更新 workspace.package.version
    sed -i.bak "/^\[workspace\.package\]/,/^\[/ s/^version = \"[^\"]*\"/version = \"$new_version\"/" Cargo.toml

    rm -f Cargo.toml.bak

    log_success "版本号已更新"
}

# 备份所有 Cargo.toml 文件
backup_cargo_toml_files() {
    log_info "备份 Cargo.toml 文件..."

    for crate_path in "${CRATE_PUBLISH_ORDER[@]}"; do
        if [[ "$crate_path" == "." ]]; then
            cp Cargo.toml Cargo.toml.release-backup
        else
            cp "$crate_path/Cargo.toml" "$crate_path/Cargo.toml.release-backup"
        fi
    done

    log_success "备份完成"
}

# 恢复所有 Cargo.toml 文件
restore_cargo_toml_files() {
    log_info "恢复 Cargo.toml 文件..."

    for crate_path in "${CRATE_PUBLISH_ORDER[@]}"; do
        if [[ "$crate_path" == "." ]]; then
            if [[ -f Cargo.toml.release-backup ]]; then
                mv Cargo.toml.release-backup Cargo.toml
            fi
        else
            if [[ -f "$crate_path/Cargo.toml.release-backup" ]]; then
                mv "$crate_path/Cargo.toml.release-backup" "$crate_path/Cargo.toml"
            fi
        fi
    done

    log_success "恢复完成"
}

# 替换 path 依赖为 version 依赖
replace_path_with_version() {
    local new_version=$1

    log_info "替换内部 path 依赖为 version 依赖..."

    # 替换 workspace dependencies
    sed -i.tmp \
        -e "s|actr-protocol = { path = \"crates/protocol\" }|actr-protocol = \"$new_version\"|g" \
        -e "s|actr-version = { path = \"crates/version\" }|actr-version = \"$new_version\"|g" \
        -e "s|actr-config = { path = \"crates/config\" }|actr-config = \"$new_version\"|g" \
        -e "s|actr-framework = { path = \"crates/framework\" }|actr-framework = \"$new_version\"|g" \
        -e "s|actr-runtime = { path = \"crates/runtime\" }|actr-runtime = \"$new_version\"|g" \
        -e "s|actr-mailbox = { path = \"crates/runtime-mailbox\" }|actr-mailbox = \"$new_version\"|g" \
        Cargo.toml
    rm -f Cargo.toml.tmp

    # 替换各 crate 内的 path 依赖
    for crate_path in "${CRATE_PUBLISH_ORDER[@]}"; do
        local cargo_toml="$crate_path/Cargo.toml"
        if [[ "$crate_path" == "." ]]; then
            cargo_toml="Cargo.toml"
        fi

        sed -i.tmp \
            -e "s|actr-protocol = { path = \"../protocol\" }|actr-protocol = \"$new_version\"|g" \
            -e "s|actr-version = { path = \"../version\" }|actr-version = \"$new_version\"|g" \
            -e "s|actr-config = { path = \"../config\" }|actr-config = \"$new_version\"|g" \
            -e "s|actr-framework = { path = \"../framework\" }|actr-framework = \"$new_version\"|g" \
            -e "s|actr-mailbox = { path = \"../runtime-mailbox\" }|actr-mailbox = \"$new_version\"|g" \
            -e "s|proto-sign = { git = \"https://github.com/actor-rtc/proto-sign\" }|proto-sign = \"0.1\"|g" \
            "$cargo_toml"
        rm -f "$cargo_toml.tmp"
    done

    log_success "依赖替换完成"
}

# 运行测试
run_tests() {
    if [[ "$NO_VERIFY" == true ]]; then
        log_warn "跳过测试（--no-verify）"
        return
    fi

    log_info "运行 workspace 测试..."
    cargo test --workspace --all-features
    log_success "测试通过"
}

# 验证单个 crate 发布
verify_crate_publish() {
    local crate_path=$1
    local crate_name=$2

    log_step "验证 $crate_name 发布配置..."

    if [[ "$crate_path" == "." ]]; then
        cargo publish --dry-run --allow-dirty -p "$crate_name"
    else
        cargo publish --dry-run --allow-dirty -p "$crate_name"
    fi

    log_success "$crate_name 验证通过"
}

# 发布单个 crate
publish_crate() {
    local crate_path=$1
    local crate_name=$2

    if [[ "$DRY_RUN" == true ]]; then
        log_warn "Dry-run 模式，跳过 $crate_name 发布"
        return
    fi

    log_step "发布 $crate_name 到 crates.io..."

    cargo publish -p "$crate_name"

    # 等待 crates.io 索引更新
    log_info "等待 crates.io 索引更新... (30 秒)"
    sleep 30

    log_success "$crate_name 发布完成"
}

# 获取 crate 名称
get_crate_name() {
    local crate_path=$1
    local cargo_toml="$crate_path/Cargo.toml"

    if [[ "$crate_path" == "." ]]; then
        cargo_toml="Cargo.toml"
    fi

    grep '^name = ' "$cargo_toml" | head -n1 | sed -E 's/name = "(.*)"/\1/'
}

# 发布所有 crates
publish_all_crates() {
    log_info "开始发布所有 crates..."
    echo

    for crate_path in "${CRATE_PUBLISH_ORDER[@]}"; do
        local actual_path="$crate_path"
        if [[ "$crate_path" == "." ]]; then
            actual_path="."
        fi

        local crate_name=$(get_crate_name "$crate_path")

        verify_crate_publish "$actual_path" "$crate_name"
        publish_crate "$actual_path" "$crate_name"
        echo
    done

    log_success "所有 crates 发布完成！"
}

# 创建 Git 标签
create_git_tag() {
    local version=$1
    local tag="v$version"

    if [[ "$DRY_RUN" == true ]]; then
        log_warn "Dry-run 模式，跳过 Git 标签"
        return
    fi

    log_info "创建 Git 标签: $tag"

    # 提交版本变更
    git add -A
    git commit -m "Release version $version"

    # 创建标签
    git tag -a "$tag" -m "Release $version"

    # 推送到远程
    git push origin HEAD
    git push origin "$tag"

    log_success "Git 标签已创建并推送: $tag"
}

# 主流程
main() {
    log_info "========================================"
    log_info "  actr Workspace Release"
    log_info "========================================"
    echo

    # 检查依赖
    if ! command -v cargo &> /dev/null; then
        log_error "未找到 cargo，请安装 Rust"
        exit 1
    fi

    # 获取版本信息
    CURRENT_VERSION=$(get_current_version)
    NEW_VERSION=$(increment_version "$CURRENT_VERSION" "$VERSION_TYPE")

    log_info "当前版本: $CURRENT_VERSION"
    log_info "目标版本: $NEW_VERSION"
    log_info "发布 crates 数量: ${#CRATE_PUBLISH_ORDER[@]}"

    if [[ "$DRY_RUN" == true ]]; then
        log_warn "Dry-run 模式：只验证，不实际发布"
    fi

    echo
    read -p "确认发布 actr workspace v$NEW_VERSION? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        log_warn "已取消发布"
        exit 0
    fi

    # 执行发布流程
    trap 'restore_cargo_toml_files' ERR EXIT

    check_git_status
    backup_cargo_toml_files
    update_version "$NEW_VERSION"

    log_info "更新 Cargo.lock..."
    cargo update --workspace

    replace_path_with_version "$NEW_VERSION"
    run_tests
    publish_all_crates

    # 恢复 Cargo.toml（但保留版本号更新）
    restore_cargo_toml_files
    update_version "$NEW_VERSION"

    log_info "更新最终 Cargo.lock..."
    cargo update --workspace

    create_git_tag "$NEW_VERSION"

    trap - ERR EXIT

    echo
    log_success "========================================"
    log_success "  actr $NEW_VERSION 发布完成！"
    log_success "========================================"
    echo

    if [[ "$DRY_RUN" == false ]]; then
        log_info "发布的 crates:"
        for crate_path in "${CRATE_PUBLISH_ORDER[@]}"; do
            local crate_name=$(get_crate_name "$crate_path")
            log_info "  - https://crates.io/crates/$crate_name"
        done
        echo
        log_info "稍等几分钟后即可使用新版本"
    fi
}

main
