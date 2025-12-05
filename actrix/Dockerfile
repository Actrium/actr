# Actrix WebRTC 辅助服务器 Docker 镜像

# 构建阶段
FROM rust:1.88-bookworm as builder

WORKDIR /build

# 缓存依赖
COPY Cargo.toml Cargo.lock ./
COPY crates crates/
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --bins
RUN rm -rf src

# 构建实际应用
COPY src src/
RUN touch src/main.rs && cargo build --release --bins

# 运行阶段
FROM debian:bookworm-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN useradd -m -u 1000 -s /bin/bash actrix

# 复制二进制文件
COPY --from=builder /build/target/release/actrix /usr/local/bin/actrix

# 创建必要目录
RUN mkdir -p /etc/actrix /var/lib/actrix/data /var/log/actrix && \
    chown -R actrix:actrix /var/lib/actrix /var/log/actrix

# 切换到非 root 用户
USER actrix
WORKDIR /var/lib/actrix

# 暴露端口
EXPOSE 8443/tcp  # HTTPS/WSS
EXPOSE 50052/tcp # KS gRPC
EXPOSE 3478/udp  # STUN/TURN

# 健康检查
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/actrix", "health-check"] || exit 1

# 入口点
ENTRYPOINT ["/usr/local/bin/actrix"]
CMD ["--config", "/etc/actrix/config.toml"]

# 标签
LABEL org.opencontainers.image.title="Actrix"
LABEL org.opencontainers.image.description="WebRTC Auxiliary Server"
LABEL org.opencontainers.image.version="0.1.0"
LABEL org.opencontainers.image.licenses="Apache-2.0"
