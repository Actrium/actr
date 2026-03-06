from __future__ import annotations

import argparse
import asyncio
import logging
import random
import string

from actr import ActrSystem, Dest, ActrType, ActrRuntimeError, WorkloadBase, DataStream, Context

logging.basicConfig(
    level=logging.INFO,
    format="[%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)

from generated.local import stream_client_pb2 as pb2
from generated import local_stream_service_actor as local_service_actor
from generated.remote.stream_register_server_python import stream_server_client  # Register RPC extensions
from generated.remote.stream_register_server_python import stream_server_pb2 as server_pb2


class LocalStreamService(local_service_actor.LocalStreamServiceHandler):
    """Local stream service implementation - orchestrates remote stream registration"""

    def __init__(self) -> None:
        self.server_type = ActrType("acme", "StreamEchoServer")
        self.received_count = {"count": 0}
        logger.info("LocalStreamService initialized")

    async def start_stream(
        self, req: pb2.StartStreamRequest, ctx: Context
    ) -> pb2.StartStreamResponse:
        message_count = req.message_count
        
        # Generate random 8-character stream_id
        stream_id = ''.join(random.choices(string.ascii_lowercase + string.digits, k=8))
        logger.info("📝 StartStream called with message_count=%s, generated stream_id=%s", message_count, stream_id)

        # 1. Discover server
        logger.info("🔍 Discovering server: %s", self.server_type)
        try:
            server_id = await ctx.discover(self.server_type)
            logger.info("✅ Discovered server: %s", server_id)
        except ActrRuntimeError as e:
            logger.error("❌ Failed to discover server: %s", e)
            return pb2.StartStreamResponse(
                success=False,
                message=f"Failed to discover server: {e}",
            )

        # 2. Register stream callback BEFORE sending register request
        async def stream_callback(stream: DataStream, sender_id) -> None:
            self.received_count["count"] += 1
            text = stream.payload().decode("utf-8", errors="replace")
            logger.info(
                "📥 Client received stream %s/%s from %s (stream_id=%s): %s",
                stream.sequence(),
                message_count,
                sender_id,
                stream.stream_id(),
                text,
            )

        logger.info("📝 Registering stream callback for stream_id=%s", stream_id)
        await ctx.register_stream(stream_id, stream_callback)

        # 3. Send RegisterStream request to server
        register_req = server_pb2.RegisterStreamRequest(
            stream_id=stream_id,
            message_count=message_count,
        )
        
        logger.info("📤 Sending RegisterStream request for stream_id=%s", stream_id)
        try:
            response_bytes = await ctx.call(
                Dest.actor(server_id),
                register_req.route_key,
                register_req,
            )
            response = server_pb2.RegisterStreamResponse.FromString(response_bytes)
            
            if response.success:
                logger.info("✅ Stream registered successfully: %s", response.message)
                
                return pb2.StartStreamResponse(
                    success=True,
                    message=f"Stream {stream_id} registered and ready to receive messages",
                )
            else:
                logger.error("❌ Stream registration failed: %s", response.message)
                return pb2.StartStreamResponse(
                    success=False,
                    message=response.message,
                )
                
        except Exception as e:
            logger.error("❌ Failed to call RegisterStream: %s", e)
            return pb2.StartStreamResponse(
                success=False,
                message=f"Exception: {e}",
            )


class LocalStreamServiceWorkload(WorkloadBase):
    def __init__(self, handler: LocalStreamService):
        self.handler = handler
        super().__init__(local_service_actor.LocalStreamServiceDispatcher())

    async def on_start(self, ctx: Context) -> None:
        logger.info("LocalStreamServiceWorkload on_start")

    async def on_stop(self, ctx: Context) -> None:
        logger.info("LocalStreamServiceWorkload on_stop")
        logger.info("📊 Total received: %s messages", self.handler.received_count["count"])


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--actr-toml", required=True)
    ap.add_argument("message_count", type=int, help="Number of stream messages to receive")
    args = ap.parse_args()
    
    logger.info("🚀 Starting StreamEchoClient...")
    system = await ActrSystem.from_toml(args.actr_toml)
    workload = LocalStreamServiceWorkload(LocalStreamService())
    node = system.attach(workload)
    ref = await node.start()
    
    logger.info("✅ StreamEchoClient started! Actor ID: %s", ref.actor_id())

    # Call local StartStream service
    start_req = pb2.StartStreamRequest(message_count=args.message_count)
    
    logger.info("📞 Calling local StartStream service...")
    try:
        response_bytes = await ref.call(start_req.route_key, start_req)
        response = pb2.StartStreamResponse.FromString(response_bytes)
        
        if response.success:
            logger.info("✅ StartStream succeeded: %s", response.message)
        else:
            logger.error("❌ StartStream failed: %s", response.message)
            return 1
    except Exception as e:
        logger.error("❌ Failed to call StartStream: %s", e)
        return 1

    # Wait for stream messages
    logger.info("⏳ Waiting for stream messages...")
    await asyncio.sleep(10)  # Wait for all messages
    
    ref.shutdown()
    await ref.wait_for_shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
