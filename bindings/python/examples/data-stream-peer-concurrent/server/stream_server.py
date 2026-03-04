# DO NOT EDIT - Generated scaffold
# TODO: Implement your business logic

from __future__ import annotations

import argparse
import asyncio
import logging
import sys
from pathlib import Path

from actr import ActrSystem, WorkloadBase, Context

# 配置日志
logging.basicConfig(
    level=logging.INFO,
    format="[%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)

# 添加 generated 目录到 Python 路径
generated_dir = Path(__file__).parent / "generated"
if str(generated_dir) not in sys.path:
    sys.path.insert(0, str(generated_dir))

# 动态导入生成的模块
from generated.local import data_stream_peer_server_pb2 as pb2
from generated import data_stream_peer_server_service_actor as actor


class StreamServerHandler(actor.StreamServerHandler):
    """
    StreamServer 业务逻辑实现
    TODO: 在此类中实现具体的 RPC 方法
    """

    def __init__(self) -> None:
        logger.info("StreamServerHandler 实例已初始化")

    async def prepare_stream(
        self, req: pb2.PrepareServerStreamRequest, ctx: Context
    ) -> pb2.PrepareServerStreamResponse:
        """
        TODO: 实现 PrepareStream RPC 方法

        Args:
            req: PrepareServerStreamRequest 请求对象
            ctx: Actor 上下文，用于服务发现或调用其他 Service

        Returns:
            PrepareServerStreamResponse 响应对象
        """
        logger.info("📝 接收到 RPC 调用: PrepareStream")

        # 示例实现逻辑:
        # return pb2.PrepareServerStreamResponse(
        #     field1="value",
        #     field2=123,
        # )

        raise NotImplementedError("方法 PrepareStream 尚未实现")


class StreamServerWorkload(WorkloadBase):
    def __init__(self, handler: StreamServerHandler):
        self.handler = handler
        super().__init__(actor.StreamServerDispatcher())

    async def on_start(self, ctx: Context) -> None:
        logger.info("🚀 工作负载 StreamServerWorkload 正在启动...")

    async def on_stop(self, ctx: Context) -> None:
        logger.info("🛑 工作负载 StreamServerWorkload 正在停止...")


async def main() -> int:
    ap = argparse.ArgumentParser(description="StreamServer Runner")
    ap.add_argument("--actr-toml", required=True, help="ACTR 配置文件路径")
    args = ap.parse_args()

    logger.info("🔧 正在初始化 StreamServer 系统...")
    system = await ActrSystem.from_toml(args.actr_toml)

    workload = StreamServerWorkload(StreamServerHandler())

    node = system.attach(workload)
    ref = await node.start()

    logger.info("✅ StreamServer 启动成功! Actor ID: %s", ref.actor_id())

    # 等待中断信号并关闭
    await ref.wait_for_ctrl_c_and_shutdown()
    logger.info("👋 StreamServer 已关闭")

    return 0


if __name__ == "__main__":
    try:
        sys_exit_code = asyncio.run(main())
        raise SystemExit(sys_exit_code)
    except KeyboardInterrupt:
        pass
