import { existsSync } from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export async function runCommand(
  command: string,
  args: string[],
  options: { cwd?: string } = {},
): Promise<void> {
  const executable = resolveExecutable(command, options.cwd ?? process.cwd());

  await new Promise<void>((resolve, reject) => {
    const child = spawn(executable, args, {
      cwd: options.cwd,
      stdio: 'inherit',
    });

    child.on('error', (error) => {
      reject(
        new Error(
          `Failed to start ${command}: ${error instanceof Error ? error.message : String(error)}`,
        ),
      );
    });
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }

      reject(
        new Error(
          `${command} failed with ${signal ? `signal ${signal}` : `exit code ${code ?? 'unknown'}`}`,
        ),
      );
    });
  });
}

function resolveExecutable(command: string, startDir: string): string {
  const local =
    findNodeModulesBin(command, startDir) ??
    findNodeModulesBin(command, path.dirname(fileURLToPath(import.meta.url)));
  return local ?? command;
}

function findNodeModulesBin(
  command: string,
  startDir: string,
): string | undefined {
  let dir = path.resolve(startDir);

  while (true) {
    const candidate = path.join(dir, 'node_modules', '.bin', command);
    if (existsSync(candidate)) {
      return candidate;
    }

    const parent = path.dirname(dir);
    if (parent === dir) {
      return undefined;
    }
    dir = parent;
  }
}
