import { mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';

import { build } from 'esbuild';

import { JcoCommandError, runJco } from './bindings.js';
import { resolveWorkloadWit } from './paths.js';

export const ASYNC_COMPONENTIZE_UPSTREAM_ISSUE =
  'https://github.com/bytecodealliance/ComponentizeJS/issues/335';

export interface ComponentizeOptions {
  output: string;
  projectDir?: string;
  bindingsDir?: string;
  wit?: string;
}

function toPublicEnvelope(): string {
  return `{
    method: envelope?.routeKey ?? '',
    routeKey: envelope?.routeKey ?? '',
    requestId: envelope?.requestId ?? '',
    payload: envelope?.payload,
  }`;
}

export function shimSource(entry: string, runtimeEntry: string): string {
  return `
	import { __dispatchDataChunk, withInvocationCtx } from ${JSON.stringify(runtimeEntry)};
	import userWorkload from ${JSON.stringify(entry)};

function activeWorkload() {
  if (!userWorkload || typeof userWorkload.dispatch !== 'function') {
    throw new Error('The workload entry must default-export defineWorkload({ dispatch(...) { ... } }).');
  }
  return userWorkload;
}

function toUint8Array(value) {
  if (value instanceof Uint8Array) {
    return value;
  }
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  if (value && typeof value.length === 'number') {
    return Uint8Array.from(value);
  }
  return new Uint8Array();
}

export const workload = {
  async dispatch(envelope, ctx) {
    return await withInvocationCtx(ctx, async () => {
      const result = await activeWorkload().dispatch(${toPublicEnvelope()}, ctx);
      return toUint8Array(result);
    });
  },

  async onStart(ctx) {
    return await withInvocationCtx(ctx, async () => {
      await activeWorkload().onStart?.(ctx);
    });
  },

  async onReady(ctx) {
    return await withInvocationCtx(ctx, async () => {
      await activeWorkload().onReady?.(ctx);
    });
  },

  async onStop(ctx) {
    return await withInvocationCtx(ctx, async () => {
      await activeWorkload().onStop?.(ctx);
    });
  },

  async onError(event, ctx) {
    return await withInvocationCtx(ctx, async () => {
      await activeWorkload().onError?.(event, ctx);
    });
  },

  async onDataChunk(chunk, sender, ctx) {
    return await withInvocationCtx(ctx, async () => {
      if (typeof activeWorkload().onDataChunk === 'function') {
        await activeWorkload().onDataChunk(chunk, sender, ctx);
        return;
      }
      await __dispatchDataChunk(chunk, sender, ctx);
    });
  },

  async onSignalingConnecting(ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onSignalingConnected(ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onSignalingDisconnected(ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onWebsocketConnecting(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onWebsocketConnected(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onWebsocketDisconnected(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onWebrtcConnecting(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onWebrtcConnected(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onWebrtcDisconnected(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onCredentialRenewed(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onCredentialExpiring(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
  async onMailboxBackpressure(_event, ctx) {
    await withInvocationCtx(ctx, async () => {});
  },
};
`;
}

export async function componentize(
  entry: string,
  options: ComponentizeOptions,
): Promise<void> {
  const wit = resolveWorkloadWit(options.wit);
  const projectDir = resolve(options.projectDir ?? '.');
  const entryPath = resolve(projectDir, entry);
  const output = resolve(options.output);
  const requireFromProject = createRequire(resolve(projectDir, 'package.json'));
  const runtimeEntry = requireFromProject.resolve('@actrium/actr-workload');
  const tempDir = await mkdtemp(resolve(tmpdir(), 'actr-workload-ts-'));
  const ownsBindingsDir = options.bindingsDir === undefined;
  const bindingsDir = options.bindingsDir
    ? resolve(projectDir, options.bindingsDir)
    : resolve(tempDir, 'bindings');
  const shimPath = resolve(tempDir, 'entry-shim.js');
  const bundlePath = resolve(tempDir, 'bundle.mjs');

  try {
    await mkdir(dirname(output), { recursive: true });
    await writeFile(shimPath, shimSource(entryPath, runtimeEntry), 'utf8');
    await build({
      entryPoints: [shimPath],
      outfile: bundlePath,
      bundle: true,
      format: 'esm',
      platform: 'node',
      target: 'es2022',
      external: ['actr:workload/host@0.2.0'],
      absWorkingDir: projectDir,
      sourcemap: false,
      logLevel: 'silent',
    });

    try {
      await runJco([
        'componentize',
        bundlePath,
        '--wit',
        wit,
        '-n',
        'actr-workload-guest-v2',
        '--disable',
        'http',
        'fetch-event',
        '-o',
        output,
        '--debug-bindings-dir',
        bindingsDir,
      ]);
    } catch (error) {
      throw explainComponentizeFailure(error);
    }
    await assertOutputFile(output);
  } finally {
    await rm(tempDir, { recursive: true, force: true });
    if (ownsBindingsDir) {
      await rm(bindingsDir, { recursive: true, force: true });
    }
  }
}

export function explainComponentizeFailure(error: unknown): Error {
  const message = error instanceof Error ? error.message : String(error);
  const output = error instanceof JcoCommandError ? error.output : '';
  const detail = `${message}\n${output}`;

  if (
    detail.includes('spidermonkey-embedding-splicer') &&
    detail.includes('not yet implemented')
  ) {
    return new Error(
      'TypeScript V2 workload componentization is blocked by missing async-func ' +
        'export support in ComponentizeJS (spidermonkey-embedding-splicer: ' +
        `"not yet implemented"). Track ${ASYNC_COMPONENTIZE_UPSTREAM_ISSUE} ` +
        'and retry after @bytecodealliance/componentize-js ships that support.',
    );
  }

  return error instanceof Error ? error : new Error(message);
}

async function assertOutputFile(output: string): Promise<void> {
  try {
    const outputStat = await stat(output);
    if (outputStat.isFile() && outputStat.size > 0) {
      return;
    }
  } catch {
    // Report a single command-level error below.
  }

  throw new Error(`jco componentize completed without writing ${output}.`);
}
