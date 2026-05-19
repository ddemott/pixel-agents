use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::Stdout;

use ratatui::crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

pub type Ratatui = Terminal<CrosstermBackend<Stdout>>;

/// RAII guard for the Ratatui terminal. Enters alternate screen + raw mode on
/// construction; restores the terminal on drop.
pub struct Tui {
    terminal: Ratatui,
}

impl Tui {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn draw<F: FnOnce(&mut ratatui::Frame)>(&mut self, render: F) -> Result<()> {
        self.terminal.draw(render)?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen,
        );
    }
}
