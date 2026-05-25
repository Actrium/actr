#!/usr/bin/env node
import { componentize } from './componentize.js';
import { generateBindings } from './bindings.js';

type Command = 'bindings' | 'componentize';

interface BindingsArgs {
  command: 'bindings';
  outDir: string;
  wit?: string;
}

interface ComponentizeArgs {
  command: 'componentize';
  entry: string;
  output: string;
  projectDir?: string;
  bindingsDir?: string;
  wit?: string;
}

const HELP = `Usage:
  actr-workload-ts bindings <out-dir> [--wit PATH]
  actr-workload-ts componentize <entry> -o <out.wasm> [--project-dir DIR] [--bindings-dir DIR] [--wit PATH]

Commands:
  bindings      Generate TypeScript guest bindings for the ACTR workload WIT.
  componentize  Bundle and componentize a TypeScript workload entry.
`;

const BINDINGS_HELP = `Usage:
  actr-workload-ts bindings <out-dir> [--wit PATH]

Options:
  --wit PATH  Override the resolved ACTR workload WIT path.
  --help      Show this help.
`;

const COMPONENTIZE_HELP = `Usage:
  actr-workload-ts componentize <entry> -o <out.wasm> [--project-dir DIR] [--bindings-dir DIR] [--wit PATH]

Options:
  -o, --output DIR      Output component path.
  --project-dir DIR     Project directory used to resolve the entry path.
  --bindings-dir DIR    Directory for generated jco debug bindings.
  --wit PATH            Override the resolved ACTR workload WIT path.
  --help                Show this help.
`;

function readOption(
  args: readonly string[],
  index: number,
  option: string,
): string {
  const value = args[index + 1];
  if (!value || value.startsWith('-')) {
    throw new Error(`${option} requires a value.`);
  }
  return value;
}

function parseBindings(args: readonly string[]): BindingsArgs {
  if (args.includes('--help') || args.includes('-h')) {
    console.log(BINDINGS_HELP);
    process.exit(0);
  }

  let wit: string | undefined;
  const positionals: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--wit') {
      wit = readOption(args, index, arg);
      index += 1;
      continue;
    }
    if (arg.startsWith('-')) {
      throw new Error(`Unknown bindings option: ${arg}`);
    }
    positionals.push(arg);
  }

  if (positionals.length !== 1) {
    throw new Error('bindings requires exactly one <out-dir> argument.');
  }

  return { command: 'bindings', outDir: positionals[0], wit };
}

function parseComponentize(args: readonly string[]): ComponentizeArgs {
  if (args.includes('--help') || args.includes('-h')) {
    console.log(COMPONENTIZE_HELP);
    process.exit(0);
  }

  let output: string | undefined;
  let projectDir: string | undefined;
  let bindingsDir: string | undefined;
  let wit: string | undefined;
  const positionals: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];

    if (arg === '-o' || arg === '--output') {
      output = readOption(args, index, arg);
      index += 1;
      continue;
    }
    if (arg === '--project-dir') {
      projectDir = readOption(args, index, arg);
      index += 1;
      continue;
    }
    if (arg === '--bindings-dir') {
      bindingsDir = readOption(args, index, arg);
      index += 1;
      continue;
    }
    if (arg === '--wit') {
      wit = readOption(args, index, arg);
      index += 1;
      continue;
    }
    if (arg.startsWith('-')) {
      throw new Error(`Unknown componentize option: ${arg}`);
    }
    positionals.push(arg);
  }

  if (positionals.length !== 1) {
    throw new Error('componentize requires exactly one <entry> argument.');
  }
  if (!output) {
    throw new Error('componentize requires -o, --output <out.wasm>.');
  }

  return {
    command: 'componentize',
    entry: positionals[0],
    output,
    projectDir,
    bindingsDir,
    wit,
  };
}

function parseArgs(argv: readonly string[]): BindingsArgs | ComponentizeArgs {
  if (argv.length === 0 || argv[0] === '--help' || argv[0] === '-h') {
    console.log(HELP);
    process.exit(0);
  }

  const command = argv[0] as Command;
  const commandArgs = argv.slice(1);

  if (command === 'bindings') {
    return parseBindings(commandArgs);
  }
  if (command === 'componentize') {
    return parseComponentize(commandArgs);
  }

  throw new Error(`Unknown command: ${argv[0]}`);
}

export async function main(argv = process.argv.slice(2)): Promise<number> {
  try {
    const args = parseArgs(argv);

    if (args.command === 'bindings') {
      await generateBindings(args.outDir, { wit: args.wit });
      return 0;
    }

    await componentize(args.entry, {
      output: args.output,
      projectDir: args.projectDir,
      bindingsDir: args.bindingsDir,
      wit: args.wit,
    });
    return 0;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error(`actr-workload-ts: ${message}`);
    return 1;
  }
}

main().then((code) => {
  process.exitCode = code;
});
