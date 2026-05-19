use anyhow::Result;
use crossterm::terminal;

pub struct RawModeGuard;

impl RawModeGuard {
    pub fn enable() -> Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}
