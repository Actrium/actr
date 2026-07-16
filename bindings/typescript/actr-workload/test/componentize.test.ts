import { describe, expect, it } from 'vitest';

import { shimSource } from '../src/componentize.js';

describe('componentize shim', () => {
  it('threads invocation context into user dispatch', () => {
    const source = shimSource('/tmp/workload.js', '/tmp/runtime.js');

    expect(source).toMatch(
      /activeWorkload\(\)\.dispatch\([\s\S]*?requestId:[\s\S]*?}, ctx\)/,
    );
  });

  it('forwards structured error events unchanged', () => {
    const source = shimSource('/tmp/workload.js', '/tmp/runtime.js');

    expect(source).toContain('activeWorkload().onError?.(event, ctx)');
    expect(source).not.toContain('errorMessage(event)');
  });
});
