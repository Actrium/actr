/**
 * WebRTC Coordinator - WebRTC 连接管理（DOM 侧）
 *
 * 负责创建和管理 RTCPeerConnection，接收 WebRTC 数据并转发
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
 * WebRTC 协调器（DOM 侧）
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

    // 监听来自 SW 的 WebRTC 命令
    this.swBridge.onMessage((message) => {
      if (message.type === 'webrtc_command') {
        this.handleWebRtcCommand(message.payload);
      }
    });
  }

  /**
   * 创建 Peer Connection
   */
  async createPeerConnection(peerId: string): Promise<void> {
    if (this.peers.has(peerId)) {
      console.warn(`[WebRTC] Peer ${peerId} already exists`);
      return;
    }

    // 创建 RTCPeerConnection
    const connection = new RTCPeerConnection(this.config);

    // TODO: 待商议：是否应该恢复预定义的 4 个 negotiated DataChannels 以优化连接速度？
    // 详见 .cursor/plans/webrtc-datachannel-negotiation-strategy.md
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

    // 监听 ICE candidate
    connection.onicecandidate = (event) => {
      if (event.candidate) {
        this.notifySW('ice_candidate', {
          peerId,
          candidate: event.candidate.toJSON(),
        });
      }
    };

    // 监听连接状态变化
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

    // 监听 ICE 连接状态
    connection.oniceconnectionstatechange = () => {
      console.log(`[WebRTC] ICE connection state: ${connection.iceConnectionState}`);
    };

    // 存储 peer 信息
    this.peers.set(peerId, {
      peerId,
      connection,
      dataChannels,
      state: connection.connectionState,
    });

    console.log(`[WebRTC] Peer connection created: ${peerId}`);
  }

  /**
   * 处理 DataChannel 消息
   */
  private handleDataChannelMessage(
    peerId: string,
    channelId: number,
    data: ArrayBuffer | Blob
  ): void {
    // 如果是 Blob，转换为 ArrayBuffer
    if (data instanceof Blob) {
      // [DEBUG] Keep for now
      console.log(
        `[WebRTC] DataChannel message received: peer=${peerId} channel=${channelId} bytes=${data.size}`
      );
      data.arrayBuffer().then((buffer) => {
        this.forwardDataChannelMessage(peerId, channelId, buffer);
      });
      return;
    }

    if (data instanceof ArrayBuffer) {
      // [DEBUG] Keep for now
      console.log(
        `[WebRTC] DataChannel message received: peer=${peerId} channel=${channelId} bytes=${data.byteLength}`
      );
      this.forwardDataChannelMessage(peerId, channelId, data);
      return;
    }

    // [DEBUG] Keep for now
    console.log(
      `[WebRTC] DataChannel message received: peer=${peerId} channel=${channelId} type=${typeof data}`
    );
  }

  /**
   * 转发 DataChannel 消息到 Service Worker
   */
  private forwardDataChannelMessage(peerId: string, channelId: number, data: ArrayBuffer): void {
    // 构造 stream ID
    const streamId = `${peerId}:${channelId}`;

    // 通过 Fast Path Forwarder 转发
    this.forwarder.forward(streamId, data);
  }

  /**
   * 处理来自 SW 的 WebRTC 命令
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
   * 设置 Remote Description
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
   * 设置 Local Description
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
   * 添加 ICE Candidate
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
   * 发送数据通过 DataChannel
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
      // Use 'as any' because RTCDataChannel.send in TS definitions doesn't yet support
      // SharedArrayBuffer-backed buffers, even though modern browsers do.
      // This avoids unnecessary memory copying.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      channel.send(data as any);
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

      // 创建专用 MessagePort 桥接：SW → port2 → port1 → DataChannel → Remote
      // 出站数据通过专用 port 零拷贝发送，不经过共享控制通道
      const mc = new MessageChannel();
      // port1 留在 DOM 侧：接收来自 SW 的出站数据，转发到 DataChannel
      // SW DataLane::PostMessage 会在 payload 前添加 5 字节传输头 [PayloadType(1)|Length(4)]，
      // 该 header 仅用于 WebSocket 多路复用，DataChannel 不需要，发送前须剥离。
      const TRANSPORT_HEADER_SIZE = 5;
      mc.port1.onmessage = (e: MessageEvent) => {
        if (channel.readyState === 'open') {
          if (e.data instanceof ArrayBuffer) {
            channel.send(e.data.slice(TRANSPORT_HEADER_SIZE));
          } else {
            // Uint8Array view – slice() copies into a plain ArrayBuffer-backed Uint8Array
            const arr = e.data as Uint8Array;
            channel.send(arr.slice(TRANSPORT_HEADER_SIZE));
          }
        }
      };
      // port2 通过 Transferable 转移给 SW → 注入 WirePool → DataLane::PostMessage
      this.swBridge.sendDataChannelPort(peerId, mc.port2);
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
   * 关闭 Peer Connection
   */
  private closePeerConnection(peerId: string): void {
    const peer = this.peers.get(peerId);
    if (!peer) {
      return;
    }

    // 关闭所有 DataChannels
    for (const channel of peer.dataChannels.values()) {
      channel.close();
    }

    // 关闭 PeerConnection
    peer.connection.close();

    this.peers.delete(peerId);
    console.log(`[WebRTC] Peer connection closed: ${peerId}`);
  }

  /**
   * 通知 Service Worker
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
   * 获取 Peer 信息
   */
  getPeerInfo(peerId: string): PeerConnectionInfo | undefined {
    return this.peers.get(peerId);
  }

  /**
   * 获取所有 Peer
   */
  getAllPeers(): PeerConnectionInfo[] {
    return Array.from(this.peers.values());
  }

  /**
   * 清理所有资源
   */
  dispose(): void {
    for (const peerId of this.peers.keys()) {
      this.closePeerConnection(peerId);
    }
    this.peers.clear();
  }
}
