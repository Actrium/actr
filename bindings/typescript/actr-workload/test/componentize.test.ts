import { describe, expect, it } from 'vitest';

import {
  ASYNC_COMPONENTIZE_UPSTREAM_ISSUE,
  explainComponentizeFailure,
  shimSource,
} from '../src/componentize.js';

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

  it('threads invocation context into registered stream callbacks', () => {
    const source = shimSource('/tmp/workload.js', '/tmp/runtime.js');

    expect(source).toContain('__dispatchDataChunk(chunk, sender, ctx)');
  });

  it('explains the known ComponentizeJS async-func blocker', () => {
    const error = explainComponentizeFailure(
      new Error(
        'spidermonkey-embedding-splicer/src/bindgen.rs: not yet implemented',
      ),
    );

    expect(error.message).toContain('missing async-func export support');
    expect(error.message).toContain(ASYNC_COMPONENTIZE_UPSTREAM_ISSUE);
  });

  it('does not mask unrelated componentization failures', () => {
    const original = new Error('unrelated jco failure');

    expect(explainComponentizeFailure(original)).toBe(original);
  });
});
