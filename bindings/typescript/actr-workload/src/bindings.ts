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
    const child = spawn('jco', args, {
      cwd: options.cwd,
      stdio: 'inherit',
      env: {
        ...process.env,
        PATH: buildPathEnv(),
      },
    });

    child.on('error', (error) => {
      reject(
        new Error(
          `Failed to start jco. Ensure @bytecodealliance/jco is installed. ${error.message}`,
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
      reject(new Error(`jco ${args.join(' ')} failed with ${reason}.`));
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
