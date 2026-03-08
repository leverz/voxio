use std::{
    cell::RefCell,
    io::Cursor,
    sync::{Arc, Mutex},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleFormat, Stream, StreamConfig,
};
use hound::{SampleFormat as WavSampleFormat, WavSpec, WavWriter};

use crate::error::{Result, VoxioError};

#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Vec<i16>,
}

pub trait AudioCapture {
    fn start(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
}

pub struct NullAudioCapture;

impl AudioCapture for NullAudioCapture {
    fn start(&mut self) -> Result<()> {
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

pub fn has_input_device() -> bool {
    cpal::default_host().default_input_device().is_some()
}

pub fn input_device_name() -> Option<String> {
    cpal::default_host()
        .default_input_device()
        .and_then(|device| device.name().ok())
}

pub struct RecordingSession {
    buffer: Arc<Mutex<Vec<i16>>>,
    stream: Stream,
    sample_rate: u32,
}

pub struct RecordingArtifact {
    pub wav_bytes: Vec<u8>,
    pub sample_count: usize,
}

thread_local! {
    static ACTIVE_RECORDING: RefCell<Option<RecordingSession>> = const { RefCell::new(None) };
}

pub fn start_recording() -> Result<RecordingSession> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| {
        VoxioError::Recording("no default input device is available".to_string())
    })?;
    let supported_config = device.default_input_config().map_err(|error| {
        VoxioError::Recording(format!("failed to read input device config: {error}"))
    })?;
    let sample_rate = supported_config.sample_rate().0;
    let channels = supported_config.channels();
    let stream_config: StreamConfig = supported_config.clone().into();
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let error_callback = |error| eprintln!("audio stream error: {error}");

    let stream = match supported_config.sample_format() {
        SampleFormat::I16 => build_i16_stream(&device, &stream_config, channels, buffer.clone(), error_callback)?,
        SampleFormat::U16 => build_u16_stream(&device, &stream_config, channels, buffer.clone(), error_callback)?,
        SampleFormat::F32 => build_f32_stream(&device, &stream_config, channels, buffer.clone(), error_callback)?,
        sample_format => {
            return Err(VoxioError::Recording(format!(
                "unsupported sample format: {sample_format:?}"
            )))
        }
    };

    stream
        .play()
        .map_err(|error| VoxioError::Recording(format!("failed to start input stream: {error}")))?;

    Ok(RecordingSession {
        buffer,
        stream,
        sample_rate,
    })
}

pub fn stop_recording(recording: RecordingSession) -> Result<RecordingArtifact> {
    drop(recording.stream);

    let samples = recording
        .buffer
        .lock()
        .expect("audio buffer mutex poisoned")
        .clone();
    if samples.is_empty() {
        return Err(VoxioError::Recording(
            "no audio samples were captured".to_string(),
        ));
    }

    let spec = WavSpec {
        channels: 1,
        sample_rate: recording.sample_rate,
        bits_per_sample: 16,
        sample_format: WavSampleFormat::Int,
    };

    let mut buffer = Cursor::new(Vec::new());
    {
        let mut writer = WavWriter::new(&mut buffer, spec)
            .map_err(|error| VoxioError::Recording(format!("failed to create wav buffer: {error}")))?;
        for sample in &samples {
            writer
                .write_sample(*sample)
                .map_err(|error| VoxioError::Recording(format!("failed to write wav sample: {error}")))?;
        }
        writer
            .finalize()
            .map_err(|error| VoxioError::Recording(format!("failed to finalize wav buffer: {error}")))?;
    }

    Ok(RecordingArtifact {
        wav_bytes: buffer.into_inner(),
        sample_count: samples.len(),
    })
}

pub fn store_active_recording(recording: RecordingSession) {
    ACTIVE_RECORDING.with(|slot| {
        *slot.borrow_mut() = Some(recording);
    });
}

pub fn take_active_recording() -> Option<RecordingSession> {
    ACTIVE_RECORDING.with(|slot| slot.borrow_mut().take())
}

pub fn clear_active_recording() {
    ACTIVE_RECORDING.with(|slot| {
        let _ = slot.borrow_mut().take();
    });
}

fn build_i16_stream(
    device: &Device,
    config: &StreamConfig,
    channels: u16,
    buffer: Arc<Mutex<Vec<i16>>>,
    error_callback: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream> {
    device
        .build_input_stream(
            config,
            move |data: &[i16], _| append_i16_samples(data, channels, &buffer),
            error_callback,
            None,
        )
        .map_err(|error| VoxioError::Recording(format!("failed to build i16 stream: {error}")))
}

fn build_u16_stream(
    device: &Device,
    config: &StreamConfig,
    channels: u16,
    buffer: Arc<Mutex<Vec<i16>>>,
    error_callback: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream> {
    device
        .build_input_stream(
            config,
            move |data: &[u16], _| {
                let mut target = buffer.lock().expect("audio buffer mutex poisoned");
                for frame in data.chunks(channels as usize) {
                    let sample = frame[0] as i32 - 32768;
                    target.push(sample as i16);
                }
            },
            error_callback,
            None,
        )
        .map_err(|error| VoxioError::Recording(format!("failed to build u16 stream: {error}")))
}

fn build_f32_stream(
    device: &Device,
    config: &StreamConfig,
    channels: u16,
    buffer: Arc<Mutex<Vec<i16>>>,
    error_callback: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream> {
    device
        .build_input_stream(
            config,
            move |data: &[f32], _| {
                let mut target = buffer.lock().expect("audio buffer mutex poisoned");
                for frame in data.chunks(channels as usize) {
                    let sample = frame[0].clamp(-1.0, 1.0);
                    target.push((sample * i16::MAX as f32) as i16);
                }
            },
            error_callback,
            None,
        )
        .map_err(|error| VoxioError::Recording(format!("failed to build f32 stream: {error}")))
}

fn append_i16_samples(data: &[i16], channels: u16, buffer: &Arc<Mutex<Vec<i16>>>) {
    let mut target = buffer.lock().expect("audio buffer mutex poisoned");
    for frame in data.chunks(channels as usize) {
        target.push(frame[0]);
    }
}
