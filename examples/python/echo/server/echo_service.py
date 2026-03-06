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
from generated.local import echo_pb2 as pb2
from generated import echo_service_actor as actor


class EchoServiceHandler(actor.EchoServiceHandler):
    """
    EchoService 业务逻辑实现
    TODO: 在此类中实现具体的 RPC 方法
    """

    def __init__(self) -> None:
        logger.info("EchoServiceHandler 实例已初始化")

    async def echo(self, req: pb2.EchoRequest, ctx: Context) -> pb2.EchoResponse:
        """
        TODO: 实现 Echo RPC 方法

        Args:
            req: EchoRequest 请求对象
            ctx: Actor 上下文，用于服务发现或调用其他 Service

        Returns:
            EchoResponse 响应对象
        """
        logger.info("📝 接收到 RPC 调用: Echo")

        # 示例实现逻辑:
        # return pb2.EchoResponse(
        #     field1="value",
        #     field2=123,
        # )

        raise NotImplementedError("方法 Echo 尚未实现")


class EchoServiceWorkload(WorkloadBase):
    def __init__(self, handler: EchoServiceHandler):
        self.handler = handler
        super().__init__(actor.EchoServiceDispatcher())

    async def on_start(self, ctx: Context) -> None:
        logger.info("🚀 工作负载 EchoServiceWorkload 正在启动...")

    async def on_stop(self, ctx: Context) -> None:
        logger.info("🛑 工作负载 EchoServiceWorkload 正在停止...")


async def main() -> int:
    ap = argparse.ArgumentParser(description="EchoService Runner")
    ap.add_argument("--actr-toml", required=True, help="ACTR 配置文件路径")
    args = ap.parse_args()

    logger.info("🔧 正在初始化 EchoService 系统...")
    system = await ActrSystem.from_toml(args.actr_toml)

    workload = EchoServiceWorkload(EchoServiceHandler())

    node = system.attach(workload)
    ref = await node.start()

    logger.info("✅ EchoService 启动成功! Actor ID: %s", ref.actor_id())

    # 等待中断信号并关闭
    await ref.wait_for_ctrl_c_and_shutdown()
    logger.info("👋 EchoService 已关闭")

    return 0


if __name__ == "__main__":
    try:
        sys_exit_code = asyncio.run(main())
        raise SystemExit(sys_exit_code)
    except KeyboardInterrupt:
        pass
