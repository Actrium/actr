/**
 * WebRTC Coordinator - WebRTC （DOM ）
 *
 *  RTCPeerConnection， WebRTC 
 */

import { ServiceWorkerBridge, WebRtcCommandPayload, WebRtcEventPayload } from './sw-bridge';
import { FastPathForwarder } from './fast-path-forwarder';

export interface WebRtcConfig {
  iceServers?: RTCIceServer[];
  iceTransportPolicy?: RTCIceTransportPolicy;
}

export interface PeerConnectionInfo {
  peerId: string;
  connection: RTCPeerConnection;
  dataChannels: Map<number, RTCDataChannel>;
  state: RTCPeerConnectionState;
}

/**
 * WebRTC （DOM ）
 */
export class WebRtcCoordinator {
  private swBridge: ServiceWorkerBridge;
  private forwarder: FastPathForwarder;
  private peers: Map<string, PeerConnectionInfo> = new Map();
  private config: WebRtcConfig;
  private laneConfigs = [
    { id: 0, label: 'RPC_RELIABLE', ordered: true, maxRetransmits: undefined },
    { id: 1, label: 'RPC_SIGNAL', ordered: true, maxRetransmits: undefined },
    { id: 2, label: 'STREAM_RELIABLE', ordered: true, maxRetransmits: undefined },
    {
      id: 3,
      label: 'STREAM_LATENCY_FIRST',
      ordered: false,
      maxRetransmits: 3,
    },
  ];
  private labelToLaneId = new Map(this.laneConfigs.map((config) => [config.label, config.id]));

  constructor(
    swBridge: ServiceWorkerBridge,
    forwarder: FastPathForwarder,
    config: WebRtcConfig = {}
  ) {
    this.swBridge = swBridge;
    this.forwarder = forwarder;
    this.config = {
      iceServers: config.iceServers || [{ urls: 'stun:stun.l.google.com:19302' }],
      iceTransportPolicy: config.iceTransportPolicy,
    };

    //  SW  WebRTC 
    this.swBridge.onMessage((message) => {
      if (message.type === 'webrtc_command') {
        this.handleWebRtcCommand(message.payload);
      }
    });
  }

  /**
   *  Peer Connection
   */
  async createPeerConnection(peerId: string): Promise<void> {
    if (this.peers.has(peerId)) {
      console.warn(`[WebRTC] Peer ${peerId} already exists`);
      return;
    }

    //  RTCPeerConnection
    const connection = new RTCPeerConnection(this.config);

    // TODO: ： 4  negotiated DataChannels ？
    //  .cursor/plans/webrtc-datachannel-negotiation-strategy.md
    // DataChannels will be created by offerer or received via ondatachannel.
    const dataChannels = new Map<number, RTCDataChannel>();

    connection.ondatachannel = (event) => {
      const channel = event.channel;
      const laneId = this.labelToLaneId.get(channel.label);
      if (laneId === undefined) {
        console.warn(`[WebRTC] Unknown DataChannel label: ${channel.label}`);
        return;
      }
      this.attachDataChannel(peerId, laneId, channel);
    };

    //  ICE candidate
    connection.onicecandidate = (event) => {
      if (event.candidate) {
        this.notifySW('ice_candidate', {
          peerId,
          candidate: event.candidate.toJSON(),
        });
      }
    };

    // 
    connection.onconnectionstatechange = () => {
      console.log(`[WebRTC] Connection state changed: ${connection.connectionState}`);
      this.notifySW('connection_state_changed', {
        peerId,
        state: connection.connectionState,
      });

      const peerInfo = this.peers.get(peerId);
      if (peerInfo) {
        peerInfo.state = connection.connectionState;
      }
    };

    //  ICE 
    connection.oniceconnectionstatechange = () => {
      console.log(`[WebRTC] ICE connection state: ${connection.iceConnectionState}`);
    };

    //  peer 
    this.peers.set(peerId, {
      peerId,
      connection,
      dataChannels,
      state: connection.connectionState,
    });

    console.log(`[WebRTC] Peer connection created: ${peerId}`);
  }

  /**
   *  DataChannel 
   *
   * The first byte of the DataChannel payload is the PayloadType indicator
   * (preserved from the transport header on the send side). We extract it
   * and use it as the virtual channel_id so the SW can route:
   *   channel 0/1 → RPC,  channel 2/3 → data_stream.
   */
  private handleDataChannelMessage(
    peerId: string,
    channelId: number,
    data: ArrayBuffer | Blob
  ): void {
    //  Blob， ArrayBuffer
    if (data instanceof Blob) {
      // [DEBUG] Keep for now
      console.log(
        `[WebRTC] DataChannel message received: peer=${peerId} channel=${channelId} bytes=${data.size}`
      );
      data.arrayBuffer().then((buffer) => {
        this.extractPayloadTypeAndForward(peerId, buffer);
      });
      return;
    }

    if (data instanceof ArrayBuffer) {
      // [DEBUG] Keep for now
      console.log(
        `[WebRTC] DataChannel message received: peer=${peerId} channel=${channelId} bytes=${data.byteLength}`
      );
      this.extractPayloadTypeAndForward(peerId, data);
      return;
    }

    // [DEBUG] Keep for now
    console.log(
      `[WebRTC] DataChannel message received: peer=${peerId} channel=${channelId} type=${typeof data}`
    );
  }

  /**
   * Extract the PayloadType prefix byte and forward the actual data to the SW.
   *
   * Wire format: [PayloadType(1) | Data(N)]
   * PayloadType values map to virtual channel IDs:
   *   0 = RPC_RELIABLE, 1 = RPC_SIGNAL, 2 = STREAM_RELIABLE, 3 = STREAM_LATENCY_FIRST
   */
  private extractPayloadTypeAndForward(peerId: string, data: ArrayBuffer): void {
    if (data.byteLength < 1) return;
    const view = new Uint8Array(data);
    const virtualChannelId = view[0]; // PayloadType byte = virtual channel_id
    const actualData = data.slice(1); // Strip the PayloadType prefix
    this.forwardDataChannelMessage(peerId, virtualChannelId, actualData);
  }

  /**
   *  DataChannel  Service Worker
   */
  private forwardDataChannelMessage(peerId: string, channelId: number, data: ArrayBuffer): void {
    //  stream ID
    const streamId = `${peerId}:${channelId}`;

    //  Fast Path Forwarder 
    this.forwarder.forward(streamId, data);
  }

  /**
   *  SW  WebRTC 
   */
  private async handleWebRtcCommand(command: WebRtcCommandPayload): Promise<void> {
    const { action, peerId } = command;

    console.log(`[WebRTC] Command ${action} for peer ${peerId}`); // [DEBUG] Keep for now
    try {
      switch (action) {
        case 'create_peer':
          await this.createPeerConnection(peerId);
          break;

        case 'set_remote_description':
          console.log('[WebRTC] Remote SDP', command.payload.sdp); // [DEBUG] Keep for now
          await this.setRemoteDescription(peerId, command.payload.sdp);
          break;

        case 'set_local_description':
          await this.setLocalDescription(peerId, command.payload.sdp);
          break;

        case 'add_ice_candidate':
          console.log('[WebRTC] ICE payload', command.payload); // [DEBUG] Keep for now
          await this.addIceCandidate(peerId, command.payload.candidate);
          break;

        case 'create_offer':
          await this.ensureOffererChannels(peerId);
          await this.createOffer(peerId);
          break;

        case 'create_ice_restart_offer':
          await this.createIceRestartOffer(peerId);
          break;

        case 'create_answer':
          await this.createAnswer(peerId);
          break;

        case 'close_peer':
          this.closePeerConnection(peerId);
          break;

        case 'send_data':
          this.sendData(peerId, command.payload.channelId, command.payload.data);
          break;

        default:
          console.warn(`[WebRTC] Unknown command: ${action}`);
      }
    } catch (error) {
      console.error(`[WebRTC] Command error:`, error);
      this.notifySW('command_error', { peerId, action, error: String(error) });
    }
  }

  /**
   *  Remote Description
   */
  private async setRemoteDescription(
    peerId: string,
    sdp: RTCSessionDescriptionInit
  ): Promise<void> {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Peer ${peerId} not found`);
    }

    await peer.connection.setRemoteDescription(sdp);
    console.log(`[WebRTC] Remote description set for ${peerId}`);
  }

  /**
   *  Local Description
   */
  private async setLocalDescription(peerId: string, sdp: RTCSessionDescriptionInit): Promise<void> {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Peer ${peerId} not found`);
    }

    await peer.connection.setLocalDescription(sdp);
    console.log(`[WebRTC] Local description set for ${peerId}`);
  }

  /**
   * Create SDP offer and notify SW.
   */
  private async createOffer(peerId: string): Promise<void> {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Peer ${peerId} not found`);
    }

    const offer = await peer.connection.createOffer();
    await peer.connection.setLocalDescription(offer);

    this.notifySW('local_description', {
      peerId,
      sdp: offer,
    });
  }

  /**
   * Create ICE restart offer and notify SW.
   * Uses the iceRestart option to generate new ICE credentials.
   */
  private async createIceRestartOffer(peerId: string): Promise<void> {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Peer ${peerId} not found`);
    }

    console.log(`[WebRTC] Creating ICE restart offer for ${peerId}`);
    const offer = await peer.connection.createOffer({ iceRestart: true });
    await peer.connection.setLocalDescription(offer);

    this.notifySW('ice_restart_local_description', {
      peerId,
      sdp: offer,
    });
  }

  /**
   * Create SDP answer and notify SW.
   */
  private async createAnswer(peerId: string): Promise<void> {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Peer ${peerId} not found`);
    }

    const answer = await peer.connection.createAnswer();
    await peer.connection.setLocalDescription(answer);

    this.notifySW('local_description', {
      peerId,
      sdp: answer,
    });
  }

  /**
   *  ICE Candidate
   */
  private async addIceCandidate(peerId: string, candidate: RTCIceCandidateInit): Promise<void> {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Peer ${peerId} not found`);
    }
    if (!candidate || typeof candidate !== 'object') {
      console.warn(`[WebRTC] ICE candidate missing for ${peerId}`);
      return;
    }
    const raw = candidate as RTCIceCandidateInit & {
      sdp_mid?: string | null;
      sdp_mline_index?: number | null;
      username_fragment?: string | null;
    };
    const normalized: RTCIceCandidateInit = {
      ...candidate,
      sdpMid: candidate.sdpMid ?? raw.sdp_mid ?? null,
      sdpMLineIndex: candidate.sdpMLineIndex ?? raw.sdp_mline_index ?? null,
      usernameFragment: candidate.usernameFragment ?? raw.username_fragment ?? null,
    };
    if (normalized.sdpMid == null && normalized.sdpMLineIndex == null) {
      normalized.sdpMLineIndex = 0;
    }
    await peer.connection.addIceCandidate(new RTCIceCandidate(normalized));
    console.log(`[WebRTC] ICE candidate added for ${peerId}`);
  }

  /**
   *  DataChannel
   */
  private sendData(peerId: string, channelId: number, data: Uint8Array): void {
    const peer = this.peers.get(peerId);
    if (!peer) {
      console.warn(`[WebRTC] sendData: Peer ${peerId} not found (may have disconnected)`);
      this.notifySW('command_error', {
        peerId,
        action: 'send_data',
        error: `Peer ${peerId} not found`,
      });
      return;
    }

    const channel = peer.dataChannels.get(channelId);
    if (!channel) {
      console.warn(`[WebRTC] sendData: DataChannel ${channelId} not found for peer ${peerId}`);
      return;
    }

    if (channel.readyState === 'open') {
      // Prepend the channelId as a PayloadType byte so the receive path
      // (extractPayloadTypeAndForward) can route it correctly.
      // Both send paths (TransportLane and send_channel_data) must use
      // the same [PayloadType(1)|Data(N)] wire format.
      const out = new Uint8Array(1 + data.byteLength);
      out[0] = channelId;
      out.set(data, 1);
      // Use 'as any' because RTCDataChannel.send in TS definitions doesn't yet support
      // SharedArrayBuffer-backed buffers, even though modern browsers do.
      // This avoids unnecessary memory copying.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      channel.send(out as any);
    } else {
      console.warn(`[WebRTC] DataChannel ${channelId} not open (state: ${channel.readyState})`);
    }
  }

  private attachDataChannel(peerId: string, laneId: number, channel: RTCDataChannel): void {
    channel.binaryType = 'arraybuffer';

    channel.onmessage = (event) => {
      this.handleDataChannelMessage(peerId, laneId, event.data);
    };

    channel.onopen = () => {
      console.log(`[WebRTC] DataChannel ${channel.label} opened`); // [DEBUG] Keep for now
      this.notifySW('datachannel_open', {
        peerId,
        channelId: laneId,
        label: channel.label,
      });

      //  MessagePort ：SW → port2 → port1 → DataChannel → Remote
      //  port ，
      //
      // NOTE: Only register the RPC_RELIABLE (lane 0) port with the SW.
      // The SW's WirePool has a single WebRTC slot, so each register_datachannel_port
      // call replaces the previous connection. By only registering lane 0, we ensure
      // all outgoing data is funnelled through a single DataChannel.
      //
      // To preserve PayloadType routing information (needed by handle_fast_path to
      // distinguish RPC vs data_stream), we keep the 1-byte PayloadType prefix and
      // strip only the 4-byte Length field from the 5-byte transport header
      // [PayloadType(1)|Length(4)].
      //
      // On the receive side, handleDataChannelMessage extracts this PayloadType byte
      // and uses it as the virtual channel_id for stream_id construction, so the SW
      // can correctly route channel 0/1 → RPC and channel 2/3 → data_stream.
      if (laneId === 0) {
        const mc = new MessageChannel();
        mc.port1.onmessage = (e: MessageEvent) => {
          if (channel.readyState === 'open') {
            // SW DataLane::PostMessage payload has a 5-byte header:
            //   [PayloadType(1) | Length(4) | Data(N)]
            // We strip the 4-byte Length field but KEEP the PayloadType byte so
            // the receiver can route correctly.  Result: [PayloadType(1) | Data(N)]
            if (e.data instanceof ArrayBuffer) {
              const src = new Uint8Array(e.data);
              const out = new Uint8Array(1 + (src.byteLength - 5));
              out[0] = src[0]; // PayloadType byte
              out.set(src.subarray(5), 1); // Data after header
              channel.send(out);
            } else {
              const src = e.data as Uint8Array;
              const out = new Uint8Array(1 + (src.length - 5));
              out[0] = src[0]; // PayloadType byte
              out.set(src.subarray(5), 1); // Data after header
              channel.send(out);
            }
          }
        };
        // port2 as Transferable to SW → WirePool → DataLane::PostMessage
        this.swBridge.sendDataChannelPort(peerId, mc.port2);
      }
    };

    channel.onclose = () => {
      console.log(`[WebRTC] DataChannel ${channel.label} closed`); // [DEBUG] Keep for now
      this.notifySW('datachannel_close', {
        peerId,
        channelId: laneId,
      });
    };

    channel.onerror = (error) => {
      console.error(`[WebRTC] DataChannel ${channel.label} error:`, error);
    };

    const peerInfo = this.peers.get(peerId);
    if (peerInfo) {
      peerInfo.dataChannels.set(laneId, channel);
    }
  }

  private async ensureOffererChannels(peerId: string): Promise<void> {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Peer ${peerId} not found`);
    }

    for (const config of this.laneConfigs) {
      if (peer.dataChannels.has(config.id)) {
        continue;
      }

      const channel = peer.connection.createDataChannel(config.label, {
        ordered: config.ordered,
        maxRetransmits: config.maxRetransmits,
      });

      this.attachDataChannel(peerId, config.id, channel);
    }
  }

  /**
   *  Peer Connection
   */
  private closePeerConnection(peerId: string): void {
    const peer = this.peers.get(peerId);
    if (!peer) {
      return;
    }

    //  DataChannels
    for (const channel of peer.dataChannels.values()) {
      channel.close();
    }

    //  PeerConnection
    peer.connection.close();

    this.peers.delete(peerId);
    console.log(`[WebRTC] Peer connection closed: ${peerId}`);
  }

  /**
   *  Service Worker
   */
  private notifySW<T extends WebRtcEventPayload['eventType']>(
    eventType: T,
    data: Extract<WebRtcEventPayload, { eventType: T }>['data']
  ): void {
    this.swBridge.sendToSW({
      type: 'webrtc_event',
      payload: {
        eventType,
        data,
      } as WebRtcEventPayload,
    });
  }

  /**
   *  Peer 
   */
  getPeerInfo(peerId: string): PeerConnectionInfo | undefined {
    return this.peers.get(peerId);
  }

  /**
   *  Peer
   */
  getAllPeers(): PeerConnectionInfo[] {
    return Array.from(this.peers.values());
  }

  /**
   * 
   */
  dispose(): void {
    for (const peerId of this.peers.keys()) {
      this.closePeerConnection(peerId);
    }
    this.peers.clear();
  }
}
