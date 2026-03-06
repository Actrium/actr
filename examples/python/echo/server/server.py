from __future__ import annotations

import argparse
import asyncio
import logging
import time

from actr import ActrSystem, WorkloadBase, Context

APP_NAME = "EchoServer"
PROJECT_NAME = "echo"
PROJECT_NAME_SNAKE = "echo"
SIGNALING_URL = "ws://localhost:8080"

logging.basicConfig(level=logging.INFO, format="[%(levelname)s] %(message)s")
logger = logging.getLogger(__name__)

from generated.local import echo_pb2 as pb2
from generated import echo_service_actor as server_actor


class EchoService(server_actor.EchoServiceHandler):
    async def echo(self, req: pb2.EchoRequest, ctx: Context) -> pb2.EchoResponse:
        logger.info("server received: %s", req.message)
        return pb2.EchoResponse(
            reply=f"Echo: {req.message}",
            timestamp=int(time.time()),
        )


class EchoServerWorkload(WorkloadBase):
    def __init__(self, handler: EchoService):
        self.handler = handler
        super().__init__(server_actor.EchoServiceDispatcher())

    async def on_start(self, ctx: Context) -> None:
        logger.info("EchoServerWorkload on_start")

    async def on_stop(self, ctx: Context) -> None:
        logger.info("EchoServerWorkload on_stop")


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--actr-toml", required=True)
    args = ap.parse_args()

    system = await ActrSystem.from_toml(args.actr_toml)
    logger.info("[%s] %s starting... actr-toml: %s", PROJECT_NAME_SNAKE, APP_NAME, args.actr_toml)
    logger.info("signaling: %s", SIGNALING_URL)
    workload = EchoServerWorkload(EchoService())
    node = system.attach(workload)
    ref = await node.start()
    logger.info("✅ %s started! Actor ID: %s", APP_NAME, ref.actor_id())

    await ref.wait_for_ctrl_c_and_shutdown()
    logger.info("%s shutting down...", APP_NAME)
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
