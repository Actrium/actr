from __future__ import annotations

import argparse
import asyncio
import logging
import sys
from pathlib import Path

from actr import ActrSystem, Dest, WorkloadBase

logging.basicConfig(
    level=logging.INFO,
    format="[%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)

from generated.local import data_stream_peer_server_pb2 as pb2
from generated.remote.data_stream_peer_concurrent_client_python import stream_client_client  # Register RPC extensions
from generated.remote.data_stream_peer_concurrent_client_python import data_stream_peer_client_pb2 as client_pb2
from generated import data_stream_peer_server_service_actor as server_actor
from actr import DataStream as StreamData, Context 


class StreamServerService(server_actor.StreamServerHandler):
    """StreamServer service implementation (custom workload)"""

    def __init__(self) -> None:
        self.received_count = {"count": 0}
        # Per-stream task spawner: 每个 stream_id 一个 Queue，保证同一 stream 串行，不同 stream 并发
        self.stream_queues: dict[str, asyncio.Queue] = {}
        logger.info("StreamServerService initialized")

    async def prepare_stream(
        self, req: pb2.PrepareServerStreamRequest, ctx:Context
    ) -> pb2.PrepareStreamResponse:
        caller = ctx.caller_id()
        if caller is None:
            raise RuntimeError("No caller_id in ctx")

        logger.info(
            "prepare_stream: stream_id=%s, expected_count=%s, caller=%s",
            req.stream_id,
            req.expected_count,
            caller,
        )

        self.received_count["count"] = 0
        stream_id = req.stream_id
        expected_count = req.expected_count

        # 为这个 stream_id 创建 Queue 和专属处理任务（如果还没有）
        if stream_id not in self.stream_queues:
            queue = asyncio.Queue()
            self.stream_queues[stream_id] = queue

            # 启动专属任务来串行处理这个 stream 的消息
            received_count = self.received_count  # Capture reference
            
            async def stream_task():
                logger.info("🚀 Started dedicated task for stream: %s", stream_id)
                
                while True:
                    try:
                        stream, sender_id = await queue.get()
                        received_count["count"] += 1
                        text = stream.payload().decode("utf-8", errors="replace")
                        logger.info(
                            "server: stream %s received %s/%s from %s (stream=%s): %s",
                            stream.stream_id(),
                            received_count["count"],
                            expected_count,
                            sender_id,
                            stream_id,
                            text,
                        )
                        queue.task_done()
                    except asyncio.CancelledError:
                        logger.info("🛑 Stream task cancelled: %s", stream_id)
                        break
                    except Exception as e:
                        logger.error("Error processing stream %s: %s", stream_id, e)

            asyncio.create_task(stream_task())

        # 注册 stream callback：快速返回，消息发送到 Queue
        async def stream_callback(stream: StreamData, sender_id) -> None:
            # 发送到该 stream 的专属 Queue（非阻塞）
            if stream_id in self.stream_queues:
                await self.stream_queues[stream_id].put((stream, sender_id))
            else:
                logger.warning("No queue found for stream: %s", stream_id)

        await ctx.register_stream(stream_id, stream_callback)

        prepare_client_req = client_pb2.PrepareClientStreamRequest(
            stream_id=stream_id,
            expected_count=expected_count,
        )

        try:
            response_bytes = await ctx.call(
                Dest.actor(caller),
                prepare_client_req.route_key,
                prepare_client_req,
            )
            prepare_client_resp = client_pb2.PrepareStreamResponse.FromString(response_bytes)

            if not prepare_client_resp.ready:
                return pb2.PrepareServerStreamResponse(
                    ready=False,
                    message=prepare_client_resp.message,
                )
        except Exception as e:
            logger.error("Failed to call PrepareClientStream: %s", e)
            return pb2.PrepareServerStreamResponse(
                ready=False,
                message=f"Failed to prepare client stream: {e}",
            )

        async def _send_stream_messages() -> None:
            logger.info("sending data stream back to client: %s", caller)
            i = 1
            while i <= expected_count:
                message = f"[server] message {i}"
                data_stream = StreamData(
                    stream_id=stream_id,
                    sequence=i,
                    payload=message.encode("utf-8"),
                )
                target = Dest.actor(caller)

                try:
                    await ctx.send_stream(target, data_stream)
                    logger.info("server sending %s/%s: %s", i, expected_count, message)
                    i += 1  # Only increment on success
                except Exception as e:
                    logger.warning("server send_data_stream failed (will retry): %s", e)
                    # Don't break, wait and retry
                    await asyncio.sleep(2.0)  # Wait longer before retry
                    continue

                if i <= expected_count:
                    await asyncio.sleep(1.0)

        asyncio.create_task(_send_stream_messages())

        return pb2.PrepareServerStreamResponse(
            ready=True,
            message=f"registered stream {stream_id} for {expected_count} messages",
        )


class StreamServerWorkload(WorkloadBase):
    def __init__(self, handler: StreamServerService):
        self.handler = handler
        super().__init__(server_actor.StreamServerDispatcher())

    async def on_start(self, ctx:Context) -> None:
        logger.info("StreamServerWorkload on_start")

    async def on_stop(self, ctx:Context) -> None:
        logger.info("StreamServerWorkload on_stop")


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--actr-toml", required=True)
    args = ap.parse_args()
    logger.info("Starting server with args: %s", args)
    # Load ActrSystem from TOML
    system = await ActrSystem.from_toml(args.actr_toml)
    # Create StreamServerWorkload
    workload = StreamServerWorkload(StreamServerService())
    # Attach workload to ActrSystem
    node = system.attach(workload)
    # Start ActrNode
    ref = await node.start()
    logger.info("Python Server started! Actor ID: %s", ref.actor_id())

    await ref.wait_for_ctrl_c_and_shutdown()
    logger.info("Server shutting down...")
    return 0

if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
