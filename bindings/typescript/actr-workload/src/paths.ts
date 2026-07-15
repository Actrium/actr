import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ENV_WIT_PATH = 'ACTR_WORKLOAD_WIT';
const REPO_WIT_PATH = 'core/framework/wit-v2/actr-workload.wit';
const PACKAGED_WIT_PATH = 'wit-v2/actr-workload.wit';

function assertExistingFile(path: string, source: string): string {
  if (!existsSync(path)) {
    throw new Error(`ACTR workload WIT from ${source} does not exist: ${path}`);
  }
  return path;
}

function findRepoWitFromCwd(): string | undefined {
  let current = resolve(process.cwd());

  while (true) {
    const candidate = resolve(current, REPO_WIT_PATH);
    if (existsSync(candidate)) {
      return candidate;
    }

    const parent = dirname(current);
    if (parent === current) {
      return undefined;
    }
    current = parent;
  }
}

function packagedWitCandidates(): string[] {
  const moduleDir = dirname(fileURLToPath(import.meta.url));
  return [
    resolve(moduleDir, PACKAGED_WIT_PATH),
    resolve(moduleDir, '..', 'src', PACKAGED_WIT_PATH),
  ];
}

export function resolveWorkloadWit(explicitPath?: string): string {
  if (explicitPath) {
    return assertExistingFile(resolve(explicitPath), 'explicit --wit/API path');
  }

  const envPath = process.env[ENV_WIT_PATH];
  if (envPath) {
    return assertExistingFile(resolve(envPath), ENV_WIT_PATH);
  }

  const repoWit = findRepoWitFromCwd();
  if (repoWit) {
    return repoWit;
  }

  for (const candidate of packagedWitCandidates()) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  throw new Error(
    [
      'ACTR workload WIT not found.',
      'Checked explicit path, ACTR_WORKLOAD_WIT, nearest core/framework/wit-v2/actr-workload.wit,',
      `and packaged fallbacks: ${packagedWitCandidates().join(', ')}`,
    ].join(' '),
  );
}
