import { mkdir } from 'node:fs/promises';
import { dirname, delimiter, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

import { resolveWorkloadWit } from './paths.js';

export interface GenerateBindingsOptions {
  wit?: string;
}

interface RunJcoOptions {
  cwd?: string;
}

const MAX_CAPTURED_OUTPUT_BYTES = 256 * 1024;

export class JcoCommandError extends Error {
  constructor(
    message: string,
    public readonly output: string,
  ) {
    super(message);
    this.name = 'JcoCommandError';
  }
}

function nodeModulesBinCandidates(): string[] {
  const candidates: string[] = [];
  let current = dirname(fileURLToPath(import.meta.url));

  while (true) {
    candidates.push(resolve(current, 'node_modules/.bin'));
    const parent = dirname(current);
    if (parent === current) {
      return candidates;
    }
    current = parent;
  }
}

function buildPathEnv(): string {
  const existingPath = process.env.PATH ?? '';
  return [...nodeModulesBinCandidates(), existingPath].join(delimiter);
}

export function runJco(
  args: readonly string[],
  options: RunJcoOptions = {},
): Promise<void> {
  return new Promise((resolvePromise, reject) => {
    let combinedOutput = '';
    const child = spawn('jco', args, {
      cwd: options.cwd,
      stdio: ['inherit', 'pipe', 'pipe'],
      env: {
        ...process.env,
        PATH: buildPathEnv(),
      },
    });

    const capture = (chunk: Buffer, destination: NodeJS.WriteStream): void => {
      destination.write(chunk);
      combinedOutput += chunk.toString('utf8');
      if (
        Buffer.byteLength(combinedOutput, 'utf8') > MAX_CAPTURED_OUTPUT_BYTES
      ) {
        combinedOutput = combinedOutput.slice(-MAX_CAPTURED_OUTPUT_BYTES);
      }
    };
    child.stdout?.on('data', (chunk: Buffer) => capture(chunk, process.stdout));
    child.stderr?.on('data', (chunk: Buffer) => capture(chunk, process.stderr));

    child.on('error', (error) => {
      reject(
        new JcoCommandError(
          `Failed to start jco. Ensure @bytecodealliance/jco is installed. ${error.message}`,
          combinedOutput,
        ),
      );
    });

    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise();
        return;
      }

      const reason =
        code === null ? `signal ${signal ?? 'unknown'}` : `exit code ${code}`;
      reject(
        new JcoCommandError(
          `jco ${args.join(' ')} failed with ${reason}.`,
          combinedOutput,
        ),
      );
    });
  });
}

export async function generateBindings(
  outDir: string,
  options: GenerateBindingsOptions = {},
): Promise<void> {
  const wit = resolveWorkloadWit(options.wit);
  const resolvedOutDir = resolve(outDir);

  await mkdir(resolvedOutDir, { recursive: true });
  await runJco(['guest-types', wit, '--out-dir', resolvedOutDir]);
}
