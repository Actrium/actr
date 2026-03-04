import { ActrRef as NativeActrRef, ActrId, ActrType, PayloadType } from '../index';

/**
 * ActrRef – reference to a running actor.
 *
 * Provides methods to interact with the actor: RPC calls, messaging, discovery, etc.
 */
export class ActrRef {
  constructor(private native: NativeActrRef) { }

  /**
   * Get the actor ID.
   *
   * @returns The actor's unique identifier
   */
  actorId(): ActrId {
    return this.native.actorId();
  }

  /**
   * Discover actors of the given type.
   *
   * @param targetType - Target actor type (manufacturer + name)
   * @param count - Number of actors to discover
   * @returns List of actor IDs
   *
   * @example
   * ```typescript
   * const servers = await actorRef.discover(
   *   { manufacturer: 'acme', name: 'EchoTwiceService' },
   *   1
   * );
   * ```
   */
  async discover(targetType: ActrType, count: number): Promise<ActrId[]> {
    return await this.native.discover(targetType, count);
  }

  /**
   * Call remote actor (RPC).
   *
   * @param routeKey - Route key (e.g. 'service.Method')
   * @param payloadType - Payload type
   * @param requestPayload - Request payload (protobuf-encoded)
   * @param timeoutMs - Timeout in ms, default 30000
   * @returns Response payload
   *
   * @example
   * ```typescript
   * const request = Buffer.from('Hello');
   * const response = await actorRef.call(
   *   'echo_twice.EchoTwiceService.EchoTwice',
   *   PayloadType.RpcReliable,
   *   request,
   *   5000
   * );
   * ```
   */
  async call(
    routeKey: string,
    payloadType: PayloadType,
    requestPayload: Buffer,
    timeoutMs: number = 30000
  ): Promise<Buffer> {
    return await this.native.call(routeKey, payloadType, requestPayload, timeoutMs);
  }

  /**
   * Type-safe RPC call using Protobuf messages.
   *
   * @param routeKey - Route key
   * @param request - Request object (must have encode() method)
   * @param responseType - Response type (must have decode() method)
   * @param payloadType - Payload type, default RpcReliable
   * @param timeoutMs - Timeout in ms, default 30000
   * @returns Decoded response object
   *
   * @example
   * ```typescript
   * import { EchoTwiceRequest, EchoTwiceResponse } from './proto/echo-twice';
   *
   * const request = new EchoTwiceRequest({ message: 'Hello' });
   * const response = await actorRef.callTyped(
   *   'echo_twice.EchoTwiceService.EchoTwice',
   *   request,
   *   EchoTwiceResponse,
   *   PayloadType.RpcReliable,
   *   5000
   * );
   * console.log(response.message);
   * ```
   */
  async callTyped<Req, Res>(
    routeKey: string,
    request: Req,
    responseType: { decode: (buf: Buffer) => Res },
    payloadType: PayloadType = PayloadType.RpcReliable,
    timeoutMs: number = 30000
  ): Promise<Res> {
    const requestBuf = (request as any).encode();
    const responseBuf = await this.call(routeKey, payloadType, requestBuf, timeoutMs);
    return responseType.decode(responseBuf);
  }

  /**
   * Send one-way message (fire-and-forget).
   *
   * @param routeKey - Route key
   * @param payloadType - Payload type
   * @param messagePayload - Message payload
   *
   * @example
   * ```typescript
   * await actorRef.tell(
   *   'notification.Service/Notify',
   *   PayloadType.RpcSignal,
   *   Buffer.from('Event occurred')
   * );
   * ```
   */
  async tell(
    routeKey: string,
    payloadType: PayloadType,
    messagePayload: Buffer
  ): Promise<void> {
    await this.native.tell(routeKey, payloadType, messagePayload);
  }

  /**
   * Trigger shutdown.
   *
   * Starts the shutdown process but does not wait for completion.
   */
  shutdown(): void {
    this.native.shutdown();
  }

  /**
   * Wait for shutdown to complete.
   *
   * Blocks until the actor has fully shut down.
   */
  async waitForShutdown(): Promise<void> {
    await this.native.waitForShutdown();
  }

  /**
   * Check if shutdown is in progress.
   *
   * @returns true if shutting down
   */
  isShuttingDown(): boolean {
    return this.native.isShuttingDown();
  }

  /**
   * Stop the actor (shutdown and wait).
   *
   * Convenience for shutdown() + waitForShutdown().
   *
   * @example
   * ```typescript
   * await actorRef.stop();
   * console.log('Actor stopped');
   * ```
   */
  async stop(): Promise<void> {
    this.shutdown();
    await this.waitForShutdown();
  }
}
