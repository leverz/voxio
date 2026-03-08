use crate::error::Result;
use cpal::traits::{DeviceTrait, HostTrait};

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
