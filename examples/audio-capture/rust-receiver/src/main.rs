//! Audio Capture Receiver
//!
//! Receives Opus audio samples via WebRTC MediaTrack and saves to WAV file.
//! Pair with the Swift AudioCaptureApp sender.

use actr_framework::{Context, MediaSample, Workload};
use actr_protocol::{ActorResult, ActrError, ActrId, RpcEnvelope};
use async_trait::async_trait;
use opus::{Channels as OpusChannels, Decoder as OpusDecoder};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 1;
const BITS_PER_SAMPLE: u16 = 16;
const OPUS_FRAME_SIZE: usize = 960;

// ── WAV writer ──────────────────────────────────────────────────────

fn write_wav(
    path: &Path,
    pcm_data: &[u8],
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
) -> std::io::Result<()> {
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_size = pcm_data.len() as u32;
    let file_size = 36 + data_size;

    let mut f = std::fs::File::create(path)?;
    // RIFF header
    f.write_all(b"RIFF")?;
    f.write_all(&file_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;
    // fmt sub-chunk
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?; // sub-chunk size
    f.write_all(&1u16.to_le_bytes())?; // PCM format
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&bits_per_sample.to_le_bytes())?;
    // data sub-chunk
    f.write_all(b"data")?;
    f.write_all(&data_size.to_le_bytes())?;
    f.write_all(pcm_data)?;
    Ok(())
}

// ── AudioRecorder ───────────────────────────────────────────────────

pub struct AudioRecorder {
    pcm_buffer: Arc<Mutex<Vec<u8>>>,
    decoder: Arc<Mutex<OpusDecoder>>,
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
}

impl AudioRecorder {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let decoder = OpusDecoder::new(SAMPLE_RATE, OpusChannels::Mono)?;
        Ok(Self {
            pcm_buffer: Arc::new(Mutex::new(Vec::new())),
            decoder: Arc::new(Mutex::new(decoder)),
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            bits_per_sample: BITS_PER_SAMPLE,
        })
    }
}

// ── NoOp Dispatcher (no RPC needed) ────────────────────────────────

pub struct AudioRecorderDispatcher;

#[async_trait]
impl actr_framework::MessageDispatcher for AudioRecorderDispatcher {
    type Workload = AudioRecorderWorkload;

    async fn dispatch<C: Context>(
        _workload: &Self::Workload,
        _envelope: RpcEnvelope,
        _ctx: &C,
    ) -> ActorResult<bytes::Bytes> {
        Err(ActrError::NotImplemented(
            "AudioRecorder has no RPC handlers".to_string(),
        ))
    }
}

// ── Workload ────────────────────────────────────────────────────────

pub struct AudioRecorderWorkload {
    inner: AudioRecorder,
}

impl AudioRecorderWorkload {
    pub fn new(inner: AudioRecorder) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Workload for AudioRecorderWorkload {
    type Dispatcher = AudioRecorderDispatcher;

    async fn on_start<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        tracing::info!("🎙️ AudioRecorder started, registering media track callback...");

        let buffer = Arc::clone(&self.inner.pcm_buffer);
        let decoder = Arc::clone(&self.inner.decoder);
        ctx.register_media_track(
            "audio-0".to_string(),
            move |sample: MediaSample, sender: ActrId| {
                let buffer = Arc::clone(&buffer);
                let decoder = Arc::clone(&decoder);
                Box::pin(async move {
                    if !sample.codec.eq_ignore_ascii_case("OPUS") {
                        return Err(ActrError::InvalidArgument(format!(
                            "Unsupported codec for audio capture: {}",
                            sample.codec
                        )));
                    }

                    let decoded_pcm = decode_opus_frame(&decoder, sample.data.as_ref()).await?;
                    let len = decoded_pcm.len();
                    buffer.lock().await.extend_from_slice(&decoded_pcm);
                    tracing::debug!(
                        "🎵 Received {} bytes from {:?}, codec={}, total buffered={}",
                        len,
                        sender,
                        sample.codec,
                        buffer.lock().await.len()
                    );
                    Ok(())
                })
            },
        )
        .await?;

        tracing::info!("✅ MediaTrack 'audio-0' registered, waiting for audio samples...");
        Ok(())
    }

    async fn on_stop<C: Context>(&self, _ctx: &C) -> ActorResult<()> {
        let pcm_data = self.inner.pcm_buffer.lock().await;
        if pcm_data.is_empty() {
            tracing::info!("🛑 No audio data received, skipping WAV write");
            return Ok(());
        }

        let output_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("recorded_audio.wav");
        match write_wav(
            &output_path,
            &pcm_data,
            self.inner.sample_rate,
            self.inner.channels,
            self.inner.bits_per_sample,
        ) {
            Ok(()) => {
                tracing::info!(
                    "💾 Saved {} bytes of PCM to {} ({:.1}s of audio)",
                    pcm_data.len(),
                    output_path.display(),
                    pcm_data.len() as f64
                        / (self.inner.sample_rate as f64
                            * self.inner.channels as f64
                            * self.inner.bits_per_sample as f64
                            / 8.0)
                );
            }
            Err(e) => {
                tracing::error!("❌ Failed to write WAV: {}", e);
            }
        }
        Ok(())
    }
}

// ── main ────────────────────────────────────────────────────────────

use actr_runtime::prelude::*;
use std::path::PathBuf;
use tracing::info;

async fn decode_opus_frame(
    decoder: &Arc<Mutex<OpusDecoder>>,
    packet: &[u8],
) -> ActorResult<Vec<u8>> {
    let mut pcm_output = vec![0_i16; OPUS_FRAME_SIZE * CHANNELS as usize];
    let decoded_samples = decoder
        .lock()
        .await
        .decode(packet, &mut pcm_output, false)
        .map_err(|e| ActrError::Internal(format!("Failed to decode Opus frame: {e}")))?;

    let mut pcm_bytes = Vec::with_capacity(decoded_samples * CHANNELS as usize * 2);
    for sample in pcm_output
        .into_iter()
        .take(decoded_samples * CHANNELS as usize)
    {
        pcm_bytes.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(pcm_bytes)
}

fn resolve_config_path() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let primary = crate_dir.join("actr.toml");
    if primary.exists() {
        primary
    } else {
        crate_dir.join("Actr.example.toml")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = resolve_config_path();
    let config = actr_config::ConfigParser::from_file(&config_path)?;
    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!("🎙️ Audio Capture Receiver starting");
    info!("📋 Using config: {}", config_path.display());

    let recorder = AudioRecorder::new()?;
    let workload = AudioRecorderWorkload::new(recorder);
    let node = unimplemented!(
        "source-defined workload examples were removed; migrate this example to a package-backed host"
    );

    let actr_ref = node.start().await?;
    info!("✅ AudioRecorder ready, waiting for audio from Swift app...");
    info!("   Press Ctrl+C to stop and save WAV file");

    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Audio Capture Receiver shut down");
    Ok(())
}
