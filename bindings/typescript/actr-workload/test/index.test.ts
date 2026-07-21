import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  ActrId,
  DataChunk,
  InvocationCtx,
  StreamCallback,
  Workload,
} from '../src/index.js';

async function loadRuntime() {
  vi.resetModules();
  const host = await import('./host-imports.js');
  host.resetHostCalls();
  const runtime = await import('../src/index.js');
  return { host, runtime };
}

function testActorId(serialNumber: number | bigint = 7n): ActrId {
  return {
    realm: { realmId: 42 },
    serialNumber,
    type: {
      manufacturer: 'acme',
      name: 'EchoService',
      version: '1.0.0',
    },
  };
}

function testChunk(overrides: Partial<DataChunk> = {}): DataChunk {
  return {
    streamId: 'stream-1',
    sequence: 3n,
    payload: new Uint8Array([1, 2, 3]),
    metadata: [{ key: 'lane', value: 'reliable' }],
    timestampMs: 1234n,
    ...overrides,
  };
}

function testInvocationCtx(ctxToken: bigint): InvocationCtx {
  return {
    ctxToken,
    selfId: testActorId(),
    requestId: 'request-1',
  };
}

describe('@actrium/actr-workload', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('returns the original workload from defineWorkload', async () => {
    const { runtime } = await loadRuntime();
    const workload: Workload = {
      dispatch: () => new Uint8Array([1]),
    };

    expect(runtime.defineWorkload(workload)).toBe(workload);
  });

  it('registers a stream and dispatches chunks to the callback', async () => {
    const { host, runtime } = await loadRuntime();
    const chunk = testChunk();
    const sender = testActorId();
    const ctx = testInvocationCtx(11n);
    const received: Array<{ chunk: DataChunk; sender: ActrId }> = [];
    const callback: StreamCallback = async (incoming, from) => {
      await Promise.resolve();
      received.push({ chunk: incoming, sender: from });
    };

    await runtime.registerStream('stream-1', callback, ctx);
    await runtime.__dispatchDataChunk(chunk, sender);

    expect(host.hostCalls.registerStream).toEqual(['stream-1']);
    expect(received).toEqual([{ chunk, sender }]);
  });

  it('forwards an explicit invocation context token to host imports', async () => {
    const { host, runtime } = await loadRuntime();

    await runtime.registerStream(
      'stream-with-context',
      () => undefined,
      testInvocationCtx(37n),
    );

    expect(host.hostCalls.ctxTokens).toEqual([37n]);
  });

  it('rejects host imports when no invocation context is active', async () => {
    const { host, runtime } = await loadRuntime();

    await expect(
      runtime.registerStream('missing-context', () => undefined),
    ).rejects.toThrow('requires an InvocationCtx');
    expect(host.hostCalls.ctxTokens).toEqual([]);
    expect(host.hostCalls.registerStream).toEqual([]);
  });

  it('unregisters streams and rejects later dispatches', async () => {
    const { host, runtime } = await loadRuntime();
    const ctx = testInvocationCtx(12n);

    await runtime.registerStream('stream-1', () => undefined, ctx);
    await runtime.unregisterStream('stream-1', ctx);

    expect(host.hostCalls.unregisterStream).toEqual(['stream-1']);
    await expect(
      runtime.__dispatchDataChunk(testChunk(), testActorId()),
    ).rejects.toThrow('No stream callback registered for stream-1');
  });

  it('sends data chunks using WIT-shaped peer destinations', async () => {
    const { host, runtime } = await loadRuntime();
    const peer = testActorId(9);
    const ctx = testInvocationCtx(13n);

    await runtime.sendDataChunk(
      { peer },
      testChunk({
        sequence: 4,
        timestampMs: 5678,
      }),
      runtime.PayloadType.StreamReliable,
      ctx,
    );

    expect(host.hostCalls.sendDataChunk).toEqual([
      {
        target: {
          tag: 'peer',
          val: {
            ...peer,
            serialNumber: 9n,
          },
        },
        chunk: {
          streamId: 'stream-1',
          sequence: 4n,
          payload: new Uint8Array([1, 2, 3]),
          metadata: [{ key: 'lane', value: 'reliable' }],
          timestampMs: 5678n,
        },
        payloadType: { tag: 'stream-reliable' },
      },
    ]);
  });

  it('sends data chunks using host and workload destinations', async () => {
    const { host, runtime } = await loadRuntime();
    const ctx = testInvocationCtx(14n);

    await runtime.sendDataChunk(
      'host',
      testChunk({ streamId: 'host-stream' }),
      runtime.PayloadType.StreamLatencyFirst,
      ctx,
    );
    await runtime.sendDataChunk(
      'workload',
      testChunk({ streamId: 'workload-stream' }),
      runtime.PayloadType.StreamReliable,
      ctx,
    );

    expect(host.hostCalls.sendDataChunk.map((call) => call.target)).toEqual([
      { tag: 'host' },
      { tag: 'workload' },
    ]);
    expect(
      host.hostCalls.sendDataChunk.map((call) => call.payloadType),
    ).toEqual([{ tag: 'stream-latency-first' }, { tag: 'stream-reliable' }]);
  });

  it('normalizes payload, metadata, sequence, and optional timestamp fields', async () => {
    const { host, runtime } = await loadRuntime();
    const buffer = new Uint8Array([9, 8, 7]).buffer;
    const ctx = testInvocationCtx(15n);

    await runtime.sendDataChunk(
      { peer: testActorId(11n) },
      {
        streamId: 'array-buffer-stream',
        sequence: 12,
        payload: buffer,
      },
      runtime.PayloadType.StreamLatencyFirst,
      ctx,
    );
    await runtime.sendDataChunk(
      { peer: testActorId(12n) },
      {
        streamId: 'array-like-stream',
        sequence: 13,
        payload: [6, 5, 4],
      },
      runtime.PayloadType.StreamReliable,
      ctx,
    );

    expect(host.hostCalls.sendDataChunk[0]?.chunk).toEqual({
      streamId: 'array-buffer-stream',
      sequence: 12n,
      payload: new Uint8Array([9, 8, 7]),
      metadata: [],
      timestampMs: undefined,
    });
    expect(host.hostCalls.sendDataChunk[1]?.chunk).toEqual({
      streamId: 'array-like-stream',
      sequence: 13n,
      payload: new Uint8Array([6, 5, 4]),
      metadata: [],
      timestampMs: undefined,
    });
  });
});
