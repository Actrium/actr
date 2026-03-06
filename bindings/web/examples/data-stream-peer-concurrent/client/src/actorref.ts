const encoder = new TextEncoder();
const decoder = new TextDecoder();

export interface CallRawCapable {
    callRaw(routeKey: string, payload: Uint8Array, timeout?: number): Promise<Uint8Array>;
}

export interface StartStreamRequest {
    client_id: string;
    stream_id: string;
    message_count: number;
}

export interface StartStreamResponse {
    accepted: boolean;
    message: string;
}

export class StreamClientActorRef {
    constructor(private readonly actor: CallRawCapable) { }

    async startStream(request: StartStreamRequest): Promise<StartStreamResponse> {
        const response = await this.actor.callRaw(
            'data_stream.StreamClient.StartStream',
            encoder.encode(JSON.stringify(request)),
            30000
        );

        return JSON.parse(decoder.decode(response)) as StartStreamResponse;
    }
}
