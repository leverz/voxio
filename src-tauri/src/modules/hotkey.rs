use crate::error::Result;

pub trait HotkeyManager {
    fn register(&mut self, accelerator: &str) -> Result<()>;
    fn unregister_all(&mut self) -> Result<()>;
}

pub struct NullHotkeyManager;

impl HotkeyManager for NullHotkeyManager {
    fn register(&mut self, _accelerator: &str) -> Result<()> {
        Ok(())
    }

    fn unregister_all(&mut self) -> Result<()> {
        Ok(())
    }
}

