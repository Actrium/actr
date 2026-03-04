import { ActrNode as NativeActrNode } from '../index';
import { ActrRef } from './ref';

/**
 * ActrNode – an actor node that has not been started yet.
 *
 * Created via ActrSystem.attach(); call start() to start the node.
 */
export class ActrNode {
  constructor(private native: NativeActrNode) { }

  /**
   * Start the node and return ActrRef.
   *
   * @returns ActrRef instance for interacting with the actor
   *
   * @example
   * ```typescript
   * const actorRef = await node.start();
   * console.log('Actor started:', actorRef.actorId());
   * ```
   */
  async start(): Promise<ActrRef> {
    const nativeRef = await this.native.start();
    return new ActrRef(nativeRef);
  }
}
