import { describe, it, expect } from 'vitest';

// Basic sanity test to ensure the module loads
describe('ACTR Module Loading', () => {
  it('should import the module successfully', () => {
    const actr = require('../index.js');
    expect(actr).toBeDefined();
    expect(actr.ActrSystem).toBeDefined();
    expect(actr.ActrNode).toBeDefined();
    expect(actr.ActrRef).toBeDefined();
    expect(actr.ContextBridge).toBeDefined();
    expect(actr.PayloadType).toBeDefined();
  });

  it('should have correct PayloadType enum values', () => {
    const { PayloadType } = require('../index.js');
    expect(PayloadType.RpcReliable).toBe(0);
    expect(PayloadType.RpcSignal).toBe(1);
    expect(PayloadType.StreamReliable).toBe(2);
    expect(PayloadType.StreamLatencyFirst).toBe(3);
    expect(PayloadType.MediaRtp).toBe(4);
  });
});
