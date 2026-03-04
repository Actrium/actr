import { ActrSystem, Context, RpcEnvelope } from '../../dist/index.js';
import type { PayloadType } from '../../dist/index.js';
import { decodeEchoResponse, encodeEchoRequest, ECHO_ROUTE_KEY } from './generated/echo.client';
import {
  decodeEchoTwiceResponse,
  encodeEchoTwiceRequest,
  ECHOTWICE_ROUTE_KEY,
} from './generated/echo_twice.client';
import { dispatchLocalActor } from './generated/local.actor';

const RPC_TIMEOUT_MS = 15000;
const RPC_PAYLOAD_TYPE: PayloadType = 0;

class EchoClientWorkload {
  async onStart(ctx: Context): Promise<void> {
    console.log('Echo client started');
  }

  async onStop(ctx: Context): Promise<void> {
    console.log('Echo client stopped');
  }

  async dispatch(ctx: Context, envelope: RpcEnvelope): Promise<Buffer> {
    console.log(`Received RPC: ${envelope.routeKey}`);
    return await dispatchLocalActor(ctx, envelope);
  }
}

async function main() {
  const system = await ActrSystem.fromConfig('./Actr.toml');
  const node = system.attach(new EchoClientWorkload());
  const actorRef = await node.start();

  console.log('Actor ID:', actorRef.actorId());
  try {
    const echoRequest = encodeEchoRequest('hello');
    const echoResponseBytes = await actorRef.call(
      ECHO_ROUTE_KEY,
      RPC_PAYLOAD_TYPE,
      echoRequest,
      RPC_TIMEOUT_MS
    );
    const echoResponse = decodeEchoResponse(echoResponseBytes);
    console.log('Echo response:', echoResponse.reply);

    const echoTwiceRequest = encodeEchoTwiceRequest('world');
    const echoTwiceResponseBytes = await actorRef.call(
      ECHOTWICE_ROUTE_KEY,
      RPC_PAYLOAD_TYPE,
      echoTwiceRequest,
      RPC_TIMEOUT_MS
    );
    const echoTwiceResponse = decodeEchoTwiceResponse(echoTwiceResponseBytes);
    console.log('EchoTwice response:', echoTwiceResponse.reply);
    await new Promise((resolve) => setTimeout(resolve, 50));
    actorRef.shutdown();
    process.exit(0);
  } catch (error) {
    actorRef.shutdown();
    await actorRef.waitForShutdown();
    throw error;
  }
}

main().catch(console.error);
