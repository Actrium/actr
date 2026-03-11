# DO NOT EDIT - Generated scaffold
# TODO: Implement your business logic

from __future__ import annotations

import argparse
import asyncio
import logging
import sys
from pathlib import Path

from actr import ActrSystem, WorkloadBase, Context

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="[%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)

# Add generated directory to Python path
generated_dir = Path(__file__).parent / "generated"
if str(generated_dir) not in sys.path:
    sys.path.insert(0, str(generated_dir))

# Dynamically import generated modules
from generated.local import echo_pb2 as pb2
from generated import echo_service_actor as actor


class EchoServiceHandler(actor.EchoServiceHandler):
    """
    EchoService business logic implementation.
    TODO: Implement specific RPC methods in this class
    """

    def __init__(self) -> None:
        logger.info("EchoServiceHandler instance initialized")

    async def echo(self, req: pb2.EchoRequest, ctx: Context) -> pb2.EchoResponse:
        """
        TODO: Implement Echo RPC method

        Args:
            req: EchoRequest request object
            ctx: Actor context for service discovery or calling other services

        Returns:
            EchoResponse response object
        """
        logger.info("Received RPC call: Echo")

        # Example implementation logic:
        # return pb2.EchoResponse(
        #     field1="value",
        #     field2=123,
        # )

        raise NotImplementedError("Echo method not yet implemented")


class EchoServiceWorkload(WorkloadBase):
    def __init__(self, handler: EchoServiceHandler):
        self.handler = handler
        super().__init__(actor.EchoServiceDispatcher())

    async def on_start(self, ctx: Context) -> None:
        logger.info("Workload EchoServiceWorkload is starting...")

    async def on_stop(self, ctx: Context) -> None:
        logger.info("Workload EchoServiceWorkload is stopping...")


async def main() -> int:
    ap = argparse.ArgumentParser(description="EchoService Runner")
    ap.add_argument("--actr-toml", required=True, help="ACTR config file path")
    args = ap.parse_args()

    logger.info("Initializing EchoService system...")
    system = await ActrSystem.from_toml(args.actr_toml)

    workload = EchoServiceWorkload(EchoServiceHandler())

    node = system.attach(workload)
    ref = await node.start()

    logger.info("EchoService started successfully! Actor ID: %s", ref.actor_id())

    # Wait for interrupt signal and shutdown
    await ref.wait_for_ctrl_c_and_shutdown()
    logger.info("EchoService shutdown")

    return 0


if __name__ == "__main__":
    try:
        sys_exit_code = asyncio.run(main())
        raise SystemExit(sys_exit_code)
    except KeyboardInterrupt:
        pass
