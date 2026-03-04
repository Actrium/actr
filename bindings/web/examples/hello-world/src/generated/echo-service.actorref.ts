/**
 * 自动生成的 ActorRef
 * 服务: EchoService
 *
 * ⚠️  请勿手动编辑此文件
 */

import { EchoRequest, EchoResponse } from './remote/echo-echo-server/echo';

/**
 * callRaw 兼容接口 (Actor 和 ActorClient 都实现了该方法)
 */
interface CallRawCapable {
  callRaw(routeKey: string, payload: Uint8Array, timeout?: number): Promise<Uint8Array>;
}

/**
 * ActrType 定义
 */
export const EchoServiceActrType = {
  manufacturer: 'acme',
  name: 'echo-client-app',
};

/**
 * EchoService 的 ActorRef 包装
 * 提供类型安全的 RPC 调用方法
 */
export class EchoServiceActorRef {
  private actor: CallRawCapable;

  constructor(actor: CallRawCapable) {
    this.actor = actor;
  }

  /**
   * 调用 Echo RPC 方法
   */
  async echo(request: EchoRequest): Promise<EchoResponse> {
    const encoded = EchoRequest.encode(request).finish();
    const responseData = await this.actor.callRaw('echo.EchoService.Echo', encoded);
    return EchoResponse.decode(responseData);
  }
}
