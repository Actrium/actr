import { ActrNode, ActrType } from '../../dist/index.js';
import type { PayloadType } from '../../dist/index.js';
import {
  decodeEchoResponse,
  encodeEchoRequest,
  ECHO_ROUTE_KEY,
} from './generated/echo.client';
import {
  decodeEchoTwiceResponse,
  encodeEchoTwiceRequest,
  ECHOTWICE_ROUTE_KEY,
} from './generated/echo_twice.client';

const RPC_TIMEOUT_MS = 15000;
const RPC_PAYLOAD_TYPE: PayloadType = 0;

async function main() {
  const node = await ActrNode.fromConfig('./actr.toml');
  const actorRef = await node.start();

  console.log('Actor ID:', actorRef.actorId());
  try {
    const [serverId] = await actorRef.discover(
      {
        manufacturer: 'actrium',
        name: 'EchoService',
        version: process.env.ECHO_ACTR_VERSION ?? '0.2.1-beta',
      } satisfies ActrType,
      1,
    );
    if (!serverId) {
      throw new Error('No EchoService target discovered');
    }

    const echoRequest = encodeEchoRequest('hello');
    const echoResponseBytes = await actorRef.call(
      serverId,
      ECHO_ROUTE_KEY,
      RPC_PAYLOAD_TYPE,
      echoRequest,
      RPC_TIMEOUT_MS,
    );
    const echoResponse = decodeEchoResponse(echoResponseBytes);
    console.log('Echo response:', echoResponse.reply);

    const echoTwiceRequest = encodeEchoTwiceRequest('world');
    const echoTwiceResponseBytes = await actorRef.call(
      serverId,
      ECHOTWICE_ROUTE_KEY,
      RPC_PAYLOAD_TYPE,
      echoTwiceRequest,
      RPC_TIMEOUT_MS,
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
