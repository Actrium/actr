# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import time

from actr_workload import Workload as WorkloadProtocol

from generated.echo_workload import EchoServiceDispatcher
from generated.local import echo_pb2 as pb2


class EchoServiceHandler:
    def echo(self, req: pb2.EchoRequest) -> pb2.EchoResponse:
        return pb2.EchoResponse(
            reply=f"echo from python: {req.message}",
            timestamp=int(time.time()),
        )


class Workload(WorkloadProtocol):
    def __init__(self) -> None:
        self._dispatcher = EchoServiceDispatcher(EchoServiceHandler())

    def dispatch(self, envelope) -> bytes:
        return self._dispatcher.dispatch(envelope)

    def on_start(self) -> None:
        return None

    def on_ready(self) -> None:
        return None

    def on_stop(self) -> None:
        return None

    def on_error(self, event) -> None:
        return None

    def on_signaling_connecting(self) -> None:
        return None

    def on_signaling_connected(self) -> None:
        return None

    def on_signaling_disconnected(self) -> None:
        return None

    def on_websocket_connecting(self, event) -> None:
        return None

    def on_websocket_connected(self, event) -> None:
        return None

    def on_websocket_disconnected(self, event) -> None:
        return None

    def on_webrtc_connecting(self, event) -> None:
        return None

    def on_webrtc_connected(self, event) -> None:
        return None

    def on_webrtc_disconnected(self, event) -> None:
        return None

    def on_credential_renewed(self, event) -> None:
        return None

    def on_credential_expiring(self, event) -> None:
        return None

    def on_mailbox_backpressure(self, event) -> None:
        return None


__all__ = ["Workload"]
