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

interface PendingPortFrame {
  channelId: number;
  payload: Uint8Array;
}

interface FragmentEntry {
  totalFrags: number;
  receivedBytes: number;
  fragments: Map<number, Uint8Array>;
}

/**
 * WebRTC （DOM ）
 */
export class WebRtcCoordinator {
  private static readonly FRAGMENT_HEADER_SIZE = 8;
  private static readonly DC_MAX_MESSAGE_SIZE = 65535;
  private static readonly DC_MAX_PAYLOAD_SIZE =
    WebRtcCoordinator.DC_MAX_MESSAGE_SIZE - WebRtcCoordinator.FRAGMENT_HEADER_SIZE;

  private swBridge: ServiceWorkerBridge;
  private forwarder: FastPathForwarder;
  private peers: Map<string, PeerConnectionInfo> = new Map();
  private pendingSends: Map<string, Map<number, Uint8Array[]>> = new Map();
  private pendingPortFrames: Map<string, PendingPortFrame[]> = new Map();
  private rpcPorts: Map<string, MessagePort> = new Map();
  private fragmentCounters: Map<string, number> = new Map();
  private reassembly: Map<string, FragmentEntry> = new Map();
  private commandQueues: Map<string, Promise<void>> = new Map();
  private config: WebRtcConfig;
  /** Dynamic TURN credentials received from SW (AIS registration) */
  private turnCredential: { username: string; credential: string } | null = null;
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
        this.enqueueWebRtcCommand(message.payload);
      } else if (message.type === 'update_turn_credential') {
        this.turnCredential = {
          username: message.payload.username,
          credential: message.payload.password,
        };
        console.log('[WebRTC] TURN credential received from SW');
      }
    });
  }

  private enqueueWebRtcCommand(command: WebRtcCommandPayload): void {
    const previous = this.commandQueues.get(command.peerId) ?? Promise.resolve();
    const next = previous
      .catch(() => undefined)
      .then(() => this.handleWebRtcCommand(command));

    this.commandQueues.set(command.peerId, next);

    void next
      .finally(() => {
        if (this.commandQueues.get(command.peerId) === next) {
          this.commandQueues.delete(command.peerId);
        }
      })
      .catch(() => undefined);
  }

  private canBindRpcPort(
    peer: PeerConnectionInfo | undefined,
    channel: RTCDataChannel | undefined
  ): boolean {
    return !!(
      peer &&
      channel &&
      channel.readyState === 'open' &&
      peer.state === 'connected' &&
      peer.connection.connectionState === 'connected'
    );
  }

  private dropRpcPort(peerId: string): void {
    const port = this.rpcPorts.get(peerId);
    if (!port) {
      return;
    }

    try {
      port.close();
    } catch {
      // Ignore close errors for stale ports.
    }
    this.rpcPorts.delete(peerId);
  }

  private reportStaleRpcPeer(
    peerId: string,
    peer: PeerConnectionInfo | undefined,
    channel: RTCDataChannel | undefined,
    reason = 'unknown'
  ): void {
    const state = channel?.readyState ?? peer?.state ?? 'missing';
    console.log(`[HostPage] staleRpcPeer peer=${peerId} reason=${reason} state=${state}`);
    this.notifySW('command_error', {
      peerId,
      action: 'send_port_frame',
      error: `datachannel_not_open:${state}`,
    });
  }

  /**
   *  Peer Connection
   */
  async createPeerConnection(peerId: string): Promise<void> {
    const existing = this.peers.get(peerId);
    if (existing) {
      const state = existing.connection.connectionState || existing.state;
      if (state === 'connected' || state === 'connecting') {
        console.warn(`[WebRTC] Peer ${peerId} already exists`);
        return;
      }

      console.warn(`[WebRTC] Replacing stale peer ${peerId} state=${state}`);
      this.closePeerConnection(peerId);
    }

    // Build ICE server list with TURN credentials injected
    const iceServers = (this.config.iceServers || []).map((server) => {
      const urls = Array.isArray(server.urls) ? server.urls : [server.urls];
      const isTurn = urls.some((url) => url.startsWith('turn:') || url.startsWith('turns:'));
      if (isTurn && this.turnCredential) {
        return {
          urls: server.urls,
          username: this.turnCredential.username,
          credential: this.turnCredential.credential,
        };
      }
      return server;
    });

    const rtcConfig: RTCConfiguration = {
      iceServers,
      iceTransportPolicy: this.config.iceTransportPolicy,
    };

    console.log('[WebRTC] RTCConfiguration:', JSON.stringify(rtcConfig));

    //  RTCPeerConnection
    const connection = new RTCPeerConnection(rtcConfig);

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

      if (connection.connectionState === 'connected') {
        const rpcChannel = dataChannels.get(0);
        if (rpcChannel && this.canBindRpcPort(this.peers.get(peerId), rpcChannel)) {
          this.bindRpcPort(peerId, rpcChannel);
        }
      } else if (
        connection.connectionState === 'disconnected' ||
        connection.connectionState === 'failed' ||
        connection.connectionState === 'closed'
      ) {
        this.dropRpcPort(peerId);
        this.dropPendingPeerFrames(peerId);
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
        this.extractFragmentAndForward(peerId, channelId, new Uint8Array(buffer));
      });
      return;
    }

    if (data instanceof ArrayBuffer) {
      // [DEBUG] Keep for now
      console.log(
        `[WebRTC] DataChannel message received: peer=${peerId} channel=${channelId} bytes=${data.byteLength}`
      );
      this.extractFragmentAndForward(peerId, channelId, new Uint8Array(data));
      return;
    }

    // [DEBUG] Keep for now
    console.log(
      `[WebRTC] DataChannel message received: peer=${peerId} channel=${channelId} type=${typeof data}`
    );
  }

  /**
   * Decode the native WebRtcDataLane frame and forward the reassembled payload to the SW.
   * Wire format: [msg_id(4) | frag_index(2) | total_frags(2) | Data(N)].
   */
  private extractFragmentAndForward(peerId: string, channelId: number, frame: Uint8Array): void {
    const payload = this.reassembleFrame(peerId, channelId, frame);
    if (!payload) return;

    const buffer = new ArrayBuffer(payload.byteLength);
    new Uint8Array(buffer).set(payload);
    this.forwardDataChannelMessage(peerId, channelId, buffer);
  }

  private reassembleFrame(
    peerId: string,
    channelId: number,
    frame: Uint8Array
  ): Uint8Array | null {
    if (frame.byteLength < WebRtcCoordinator.FRAGMENT_HEADER_SIZE) {
      console.warn(`[WebRTC] Dropping short DataChannel frame: ${frame.byteLength} bytes`);
      return null;
    }

    const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
    const msgId = view.getUint32(0, false);
    const fragIndex = view.getUint16(4, false);
    const totalFrags = view.getUint16(6, false);
    const payload = frame.subarray(WebRtcCoordinator.FRAGMENT_HEADER_SIZE);

    if (totalFrags === 0 || fragIndex >= totalFrags) {
      console.warn(
        `[WebRTC] Dropping invalid DataChannel fragment: msg=${msgId} index=${fragIndex} total=${totalFrags}`
      );
      return null;
    }

    if (totalFrags === 1) {
      return payload;
    }

    const key = `${peerId}:${channelId}:${msgId}`;
    let entry = this.reassembly.get(key);
    if (!entry) {
      entry = {
        totalFrags,
        receivedBytes: 0,
        fragments: new Map(),
      };
      this.reassembly.set(key, entry);
    }

    if (!entry.fragments.has(fragIndex)) {
      const copy = new Uint8Array(payload);
      entry.fragments.set(fragIndex, copy);
      entry.receivedBytes += copy.byteLength;
    }

    if (entry.fragments.size !== entry.totalFrags) {
      return null;
    }

    const complete = new Uint8Array(entry.receivedBytes);
    let offset = 0;
    for (let i = 0; i < entry.totalFrags; i++) {
      const fragment = entry.fragments.get(i);
      if (!fragment) return null;
      complete.set(fragment, offset);
      offset += fragment.byteLength;
    }

    this.reassembly.delete(key);
    return complete;
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
          await this.createAnswer(peerId, command.payload?.sdpExchangeId);
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
  private async createAnswer(peerId: string, sdpExchangeId?: string): Promise<void> {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Peer ${peerId} not found`);
    }

    const answer = await peer.connection.createAnswer();
    await peer.connection.setLocalDescription(answer);

    this.notifySW('local_description', {
      peerId,
      sdp: answer,
      ...(sdpExchangeId ? { sdpExchangeId } : {}),
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
      this.queuePendingSend(peerId, channelId, data);
      return;
    }

    if (channel.readyState === 'open') {
      this.sendFramedData(peerId, channelId, channel, data);
    } else if (channel.readyState === 'connecting') {
      this.queuePendingSend(peerId, channelId, data);
    } else {
      console.warn(`[WebRTC] DataChannel ${channelId} not open (state: ${channel.readyState})`);
      this.dropPendingPeerFrames(peerId);
      this.notifySW('command_error', {
        peerId,
        action: 'send_data',
        error: `datachannel_not_open:${channel.readyState}`,
      });
    }
  }

  private queuePendingSend(peerId: string, channelId: number, data: Uint8Array): void {
    let byChannel = this.pendingSends.get(peerId);
    if (!byChannel) {
      byChannel = new Map();
      this.pendingSends.set(peerId, byChannel);
    }
    let queue = byChannel.get(channelId);
    if (!queue) {
      queue = [];
      byChannel.set(channelId, queue);
    }
    queue.push(new Uint8Array(data));
  }

  private queuePendingPortFrameForChannel(
    peerId: string,
    channelId: number,
    payload: Uint8Array
  ): void {
    const queue = this.pendingPortFrames.get(peerId) ?? [];
    queue.push({ channelId, payload: new Uint8Array(payload) });
    this.pendingPortFrames.set(peerId, queue);
  }

  private dropPendingPeerFrames(peerId: string): void {
    this.pendingSends.delete(peerId);
    this.pendingPortFrames.delete(peerId);
  }

  private bindRpcPort(peerId: string, channel: RTCDataChannel): void {
    const peer = this.peers.get(peerId);
    if (!this.canBindRpcPort(peer, channel)) {
      console.log(
        `[HostPage] skipBindRpcPort peer=${peerId} state=${peer?.state ?? 'missing'} dc=${channel.readyState}`
      );
      return;
    }

    const previousPort = this.rpcPorts.get(peerId);
    if (previousPort) {
      try {
        previousPort.close();
      } catch {
        // Ignore close errors for stale ports.
      }
    }

    const mc = new MessageChannel();
    mc.port1.onmessage = (e: MessageEvent) => {
      const src = this.toUint8Array(e.data);
      if (src.byteLength < 5) {
        console.warn(`[WebRTC] Dropping short SW transport frame: ${src.byteLength} bytes`);
        return;
      }

      const payloadType = src[0];
      const payload = src.subarray(5); // Strip SW transport header [PayloadType(1)|Length(4)].
      const targetChannel = peer?.dataChannels.get(payloadType);

      if (targetChannel?.readyState === 'open') {
        this.sendFramedData(peerId, payloadType, targetChannel, payload);
      } else if (targetChannel?.readyState === 'connecting') {
        this.queuePendingPortFrameForChannel(peerId, payloadType, payload);
      } else {
        this.dropPendingPeerFrames(peerId);
        this.notifySW('command_error', {
          peerId,
          action: 'send_port_frame',
          error: `datachannel_not_open:${targetChannel?.readyState ?? 'missing'}`,
        });
      }
    };

    this.rpcPorts.set(peerId, mc.port1);
    this.swBridge.sendDataChannelPort(peerId, mc.port2);
    this.flushPendingPortFrames(peerId, channel);
  }

  private flushPendingSends(peerId: string, channelId: number): void {
    const byChannel = this.pendingSends.get(peerId);
    const queue = byChannel?.get(channelId);
    if (!queue || queue.length === 0) {
      return;
    }

    byChannel?.delete(channelId);
    if (byChannel && byChannel.size === 0) {
      this.pendingSends.delete(peerId);
    }

    for (const payload of queue) {
      this.sendData(peerId, channelId, payload);
    }
  }

  private flushPendingPortFrames(peerId: string, channel: RTCDataChannel): void {
    const queue = this.pendingPortFrames.get(peerId);
    if (!queue || queue.length === 0 || channel.readyState !== 'open') {
      return;
    }

    this.pendingPortFrames.delete(peerId);
    for (const frame of queue) {
      const targetChannel = this.peers.get(peerId)?.dataChannels.get(frame.channelId);
      if (!targetChannel || targetChannel.readyState !== 'open') {
        this.queuePendingPortFrameForChannel(peerId, frame.channelId, frame.payload);
        continue;
      }
      this.sendFramedData(peerId, frame.channelId, targetChannel, frame.payload);
    }
  }

  private toUint8Array(data: ArrayBuffer | ArrayBufferView): Uint8Array {
    if (data instanceof ArrayBuffer) {
      return new Uint8Array(data);
    }
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }

  private sendFramedData(
    peerId: string,
    channelId: number,
    channel: RTCDataChannel,
    payload: Uint8Array
  ): void {
    for (const frame of this.createFrames(peerId, channelId, payload)) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      channel.send(frame as any);
    }
  }

  private createFrames(peerId: string, channelId: number, payload: Uint8Array): Uint8Array[] {
    const totalFrags = Math.max(
      1,
      Math.ceil(payload.byteLength / WebRtcCoordinator.DC_MAX_PAYLOAD_SIZE)
    );
    if (totalFrags > 0xffff) {
      throw new Error(`DataChannel payload too large: ${payload.byteLength} bytes`);
    }

    const counterKey = `${peerId}:${channelId}`;
    const msgId = this.fragmentCounters.get(counterKey) ?? 0;
    this.fragmentCounters.set(counterKey, (msgId + 1) >>> 0);

    const frames: Uint8Array[] = [];
    for (let fragIndex = 0; fragIndex < totalFrags; fragIndex++) {
      const start = fragIndex * WebRtcCoordinator.DC_MAX_PAYLOAD_SIZE;
      const end = Math.min(start + WebRtcCoordinator.DC_MAX_PAYLOAD_SIZE, payload.byteLength);
      const chunk = payload.subarray(start, end);
      const frame = new Uint8Array(WebRtcCoordinator.FRAGMENT_HEADER_SIZE + chunk.byteLength);
      const view = new DataView(frame.buffer);
      view.setUint32(0, msgId, false);
      view.setUint16(4, fragIndex, false);
      view.setUint16(6, totalFrags, false);
      frame.set(chunk, WebRtcCoordinator.FRAGMENT_HEADER_SIZE);
      frames.push(frame);
    }

    return frames;
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
      // NOTE: Only the RPC_RELIABLE lane registers a MessagePort with the SW.
      // The SW transport header still carries PayloadType, and the DOM bridge
      // uses it to select the actual RTCDataChannel before wrapping bytes in
      // the native WebRtcDataLane fragment frame.
      //
      // Receive-side routing uses the RTCDataChannel lane id after decoding the
      // fragment frame, matching the native runtime's wire format.
      if (laneId === 0) {
        this.bindRpcPort(peerId, channel);
      }

      this.flushPendingSends(peerId, laneId);
    };

    channel.onclose = () => {
      console.log(`[WebRTC] DataChannel ${channel.label} closed`); // [DEBUG] Keep for now
      if (laneId === 0) {
        this.dropRpcPort(peerId);
      }
      this.dropPendingPeerFrames(peerId);
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
    this.dropRpcPort(peerId);
    this.dropPendingPeerFrames(peerId);
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
