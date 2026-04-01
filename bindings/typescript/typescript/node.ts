import { ActrNode as NativeActrNode } from '../index';
import { ActrRef } from './ref';

/**
 * ActrNode – an actor node that has not been started yet.
 *
 * Create it from config with `ActrNode.fromConfig()`, then call `start()`.
 */
export class ActrNode {
  constructor(private native: NativeActrNode) {}

  /**
   * Create a client-only ActrNode from `manifest.toml`.
   * The sibling `actr.toml` in the same directory is loaded automatically.
   *
   * @param configPath - Path to manifest.toml
   * @returns ActrNode instance
   */
  static async fromConfig(configPath: string): Promise<ActrNode> {
    const nativeNode = await NativeActrNode.fromFile(configPath);
    return new ActrNode(nativeNode);
  }

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
