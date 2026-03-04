import { ActrSystem, Workload, Context, RpcEnvelope } from '../../dist/index.js';

const ECHO_TWICE_ROUTE_KEY = 'echo_twice.EchoTwiceService.EchoTwice';

type EchoTwiceRequest = {
  message: string;
};

type EchoTwiceResponse = {
  reply: string;
  timestamp: bigint;
};

function decodeEchoTwiceRequest(buffer: Buffer): EchoTwiceRequest {
  let offset = 0;
  let message = '';

  while (offset < buffer.length) {
    const tagResult = decodeVarint(buffer, offset);
    const tag = Number(tagResult.value);
    offset += tagResult.length;

    const fieldNumber = tag >> 3;
    const wireType = tag & 0x07;

    if (wireType === 2) {
      const lengthResult = decodeVarint(buffer, offset);
      const length = varintToNumber(lengthResult.value, 'length');
      offset += lengthResult.length;

      const end = offset + length;
      const value = buffer.subarray(offset, end);
      offset = end;

      if (fieldNumber === 1) {
        message = value.toString('utf8');
      }
      continue;
    }

    if (wireType === 0) {
      const valueResult = decodeVarint(buffer, offset);
      offset += valueResult.length;
      void valueResult;
      continue;
    }

    throw new Error(`Unsupported wire type: ${wireType}`);
  }

  return { message };
}

function encodeEchoTwiceResponse(message: EchoTwiceResponse): Buffer {
  const parts: Buffer[] = [];

  if (message.reply) {
    const replyBytes = Buffer.from(message.reply, 'utf8');
    parts.push(Buffer.from([0x0a]));
    parts.push(encodeVarint(replyBytes.length));
    parts.push(replyBytes);
  }

  if (message.timestamp && message.timestamp !== 0n) {
    parts.push(Buffer.from([0x10]));
    parts.push(encodeVarintBigint(message.timestamp));
  }

  return Buffer.concat(parts);
}

function encodeVarint(value: number): Buffer {
  let v = value >>> 0;
  const bytes: number[] = [];
  while (v >= 0x80) {
    bytes.push((v & 0x7f) | 0x80);
    v >>>= 7;
  }
  bytes.push(v);
  return Buffer.from(bytes);
}

function encodeVarintBigint(value: bigint): Buffer {
  let v = value;
  const bytes: number[] = [];
  while (v >= 0x80n) {
    bytes.push(Number((v & 0x7fn) | 0x80n));
    v >>= 7n;
  }
  bytes.push(Number(v));
  return Buffer.from(bytes);
}

function decodeVarint(buffer: Buffer, offset: number): { value: bigint; length: number } {
  let result = 0n;
  let shift = 0n;
  let i = 0;

  while (offset + i < buffer.length) {
    const byte = BigInt(buffer[offset + i]);
    result |= (byte & 0x7fn) << shift;
    i += 1;
    if ((byte & 0x80n) === 0n) {
      return { value: result, length: i };
    }
    shift += 7n;
  }

  throw new Error('Invalid varint: buffer ended unexpectedly');
}

function varintToNumber(value: bigint, label: string): number {
  if (value > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error(`Varint ${label} exceeds safe integer range`);
  }
  return Number(value);
}

class EchoTwiceServerWorkload implements Workload {
  async onStart(ctx: Context): Promise<void> {
    console.log('EchoTwice server started');
  }

  async onStop(ctx: Context): Promise<void> {
    console.log('EchoTwice server stopped');
  }

  async dispatch(ctx: Context, envelope: RpcEnvelope): Promise<Buffer> {
    if (envelope.routeKey === ECHO_TWICE_ROUTE_KEY) {
      const request = decodeEchoTwiceRequest(envelope.payload);
      const reply = `${request.message}${request.message}`;
      const response = encodeEchoTwiceResponse({
        reply,
        timestamp: BigInt(Date.now()),
      });
      console.log('EchoTwice payload:', request.message);
      return response;
    }
    throw new Error(`Unknown route: ${envelope.routeKey}`);
  }
}

async function main() {
  const system = await ActrSystem.fromConfig('./Actr.toml');
  const node = system.attach(new EchoTwiceServerWorkload());
  const actorRef = await node.start();

  console.log('EchoTwice server started with ID:', actorRef.actorId());
  await actorRef.waitForShutdown();
}

main().catch(console.error);
