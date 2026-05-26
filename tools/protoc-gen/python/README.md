# protoc-gen-python

Python code generation targets `actr-workload` Component Model workloads.

## Current Status

- Generates protobuf `*_pb2.py` modules via `protoc --python_out`.
- Generates typed workload dispatchers for local protobuf services.
- Does not generate clients, remote proxies, or code that imports the removed
  legacy `actr` Python runtime package.

## Generated Shape

For a local service method such as:

```proto
service EchoService {
  rpc Echo(EchoRequest) returns (EchoResponse);
}
```

the plugin generates a dispatcher that:

1. reads `envelope.route_key`,
2. decodes `envelope.payload` into `EchoRequest`,
3. calls the user handler method,
4. serializes the returned `EchoResponse`.

User code imports `actr_workload.Workload` and implements the generated handler
methods in `workload.py`.
