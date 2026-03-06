from __future__ import annotations

import argparse
import asyncio
import logging

from actr import ActrSystem, Dest, WorkloadBase, DataStream, Context

logging.basicConfig(
    level=logging.INFO,
    format="[%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)

from generated.local import stream_server_pb2 as pb2
from generated import stream_server_actor as server_actor


class StreamServerService(server_actor.StreamServerHandler):
    """StreamServer service implementation"""

    def __init__(self) -> None:
        self.active_streams: dict[str, str] = {}  # stream_id -> client_id mapping
        logger.info("StreamServerService initialized")

    async def register_stream(
        self, req: pb2.RegisterStreamRequest, ctx: Context
    ) -> pb2.RegisterStreamResponse:
        stream_id = req.stream_id
        message_count = req.message_count
        caller = ctx.caller_id()
        
        if caller is None:
            return pb2.RegisterStreamResponse(
                success=False,
                message="No caller_id in context",
            )

        logger.info(
            "📝 Registering stream: stream_id=%s, message_count=%s, caller=%s",
            stream_id,
            message_count,
            caller,
        )

        # Store the active stream
        self.active_streams[stream_id] = str(caller)

        # Start sending stream messages to client
        async def _send_stream_messages() -> None:
            logger.info("🚀 Starting to send %s stream messages for stream_id=%s", message_count, stream_id)
            
            for i in range(1, message_count + 1):  # Send message_count messages
                message = f"[server] Stream message {i} for {stream_id}"
                data_stream = DataStream(
                    stream_id=stream_id,
                    sequence=i,
                    payload=message.encode("utf-8"),
                )
                target = Dest.actor(caller)

                try:
                    await ctx.send_stream(target, data_stream)
                    logger.info("📤 Sent stream %s/%s: %s", i, message_count, message)
                except Exception as e:
                    logger.error("❌ Failed to send stream: %s", e)
                    break

                await asyncio.sleep(1.0)
            
            logger.info("✅ Finished sending all stream messages for %s", stream_id)

        # Start sending in background
        asyncio.create_task(_send_stream_messages())

        return pb2.RegisterStreamResponse(
            success=True,
            message=f"Stream {stream_id} registered successfully",
        )


class StreamServerWorkload(WorkloadBase):
    def __init__(self, handler: StreamServerService):
        self.handler = handler
        super().__init__(server_actor.StreamServerDispatcher())

    async def on_start(self, ctx: Context) -> None:
        logger.info("StreamServerWorkload on_start")

    async def on_stop(self, ctx: Context) -> None:
        logger.info("StreamServerWorkload on_stop")


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--actr-toml", required=True)
    args = ap.parse_args()
    
    logger.info("🚀 Starting StreamEchoServer...")
    system = await ActrSystem.from_toml(args.actr_toml)
    workload = StreamServerWorkload(StreamServerService())
    node = system.attach(workload)
    ref = await node.start()
    
    logger.info("✅ StreamEchoServer started! Actor ID: %s", ref.actor_id())

    await ref.wait_for_ctrl_c_and_shutdown()
    logger.info("Server shutting down...")
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
