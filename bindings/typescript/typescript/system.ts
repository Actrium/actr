import { ActrSystem as NativeActrSystem } from '../index';
import { ActrNode } from './node';
import { ActrId, Context, RpcEnvelope, PayloadType } from './types';
import { Workload } from './workload';

/**
 * ActrSystem – entry point for the ACTR system.
 *
 * Use it to create and configure the ACTR system, then attach a workload to create actor nodes.
 */
export class ActrSystem {
  private native: NativeActrSystem;

  private constructor(native: NativeActrSystem) {
    this.native = native;
  }

  /**
   * Create ActrSystem from a TOML config file path.
   *
   * @param configPath - Path to Actr.toml
   * @returns ActrSystem instance
   *
   * @example
   * ```typescript
   * const system = await ActrSystem.fromConfig('./Actr.toml');
   * ```
   */
  static async fromConfig(configPath: string): Promise<ActrSystem> {
    const native = await NativeActrSystem.fromFile(configPath);
    return new ActrSystem(native);
  }

  /**
   * Attach a workload and create ActrNode.
   *
   * @param workload - Object implementing the Workload interface
   * @returns ActrNode instance
   *
   * @example
   * ```typescript
   * class MyWorkload implements Workload {
   *   async onStart(ctx) { ... }
   *   async onStop(ctx) { ... }
   *   async dispatch(ctx, envelope) { ... }
   * }
   *
   * const node = system.attach(new MyWorkload());
   * ```
   */
  attach(workload: Workload): ActrNode {
    const nativeNode = this.native.attach({
      onStart: async (err: unknown, ctx: Context) => {
        if (err) {
          throw err;
        }
        this.wrapContext(ctx);
        await workload.onStart(ctx);
      },
      onStop: async (err: unknown, ctx: Context) => {
        if (err) {
          throw err;
        }
        this.wrapContext(ctx);
        await workload.onStop(ctx);
      },
      dispatch: async (
        err: unknown,
        ctx: Context,
        envelope: RpcEnvelope
      ) => {
        if (err) {
          throw err;
        }
        this.wrapContext(ctx);
        return await workload.dispatch(ctx, envelope);
      },
    });
    return new ActrNode(nativeNode);
  }

  private wrapContext(ctx: Context): void {
    if (!ctx.call) {
      ctx.call = (target: ActrId, routeKey: string, payload: Buffer) => {
        return ctx.callRaw(target, routeKey, PayloadType.RpcReliable, payload, 30000);
      };
    }
  }
}
