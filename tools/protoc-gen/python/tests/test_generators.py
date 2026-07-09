import unittest
from types import SimpleNamespace

from framework_codegen_python.generators import generate_local_workload_module


class GeneratorTests(unittest.TestCase):
    def test_nested_rpc_types_keep_their_owner_relative_path(self) -> None:
        method = SimpleNamespace(
            name="Call",
            input_type=".ask.Outer.InnerRequest",
            output_type=".ask.Outer.InnerResponse",
            client_streaming=False,
            server_streaming=False,
        )
        service = SimpleNamespace(name="ClientService", method=[method])
        type_to_owner = {
            "ask.Outer.InnerRequest": (
                "ask",
                "remote/ask/ask.proto",
                ("Outer", "InnerRequest"),
            ),
            "ask.Outer.InnerResponse": (
                "ask",
                "remote/ask/ask.proto",
                ("Outer", "InnerResponse"),
            ),
        }

        generated = generate_local_workload_module(
            "client",
            "local/client.proto",
            [service],
            type_to_owner,
        )

        self.assertIn(
            "remote_ask_ask_pb2.Outer.InnerRequest.FromString",
            generated["content"],
        )
        self.assertIn(
            "isinstance(resp, remote_ask_ask_pb2.Outer.InnerResponse)",
            generated["content"],
        )


if __name__ == "__main__":
    unittest.main()
