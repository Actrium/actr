import { Context, RpcEnvelope } from './types';

/**
 * Workload interface – implement this to define actor behaviour.
 */
export interface Workload {
  /**
   * Lifecycle hook: called when the workload starts.
   *
   * @param ctx - Context for calling remote actors
   */
  onStart(ctx: Context): Promise<void>;

  /**
   * Lifecycle hook: called when the workload stops.
   *
   * @param ctx - Context for calling remote actors
   */
  onStop(ctx: Context): Promise<void>;

  /**
   * Dispatch an incoming RPC message.
   *
   * @param ctx - Context for calling remote actors
   * @param envelope - Incoming RPC envelope (routeKey, payload, requestId)
   * @returns Response payload (protobuf-encoded Buffer)
   */
  dispatch(ctx: Context, envelope: RpcEnvelope): Promise<Buffer>;
}
