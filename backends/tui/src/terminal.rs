use std::{
    convert::TryFrom,
    io::{self, Stdout, Write},
};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute, queue,
    style::PrintStyledContent,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::{
    error::TuiError,
    renderer::{RenderFrame, RenderLine},
};

/// Represents the concrete output target the terminal backend writes to.
#[derive(Debug)]
enum TerminalTarget {
    Stdout {
        handle: Stdout,
        raw_mode: bool,
        alternate_screen: bool,
        cursor_hidden: bool,
    },
    Buffer(Vec<u8>),
}

impl TerminalTarget {
    fn stdout() -> Result<Self, TuiError> {
        let mut handle = io::stdout();
        terminal::enable_raw_mode()?;
        execute!(handle, EnterAlternateScreen, Hide)?;
        Ok(Self::Stdout {
            handle,
            raw_mode: true,
            alternate_screen: true,
            cursor_hidden: true,
        })
    }

    const fn buffered() -> Self {
        Self::Buffer(Vec::new())
    }

    fn write_frame(&mut self, frame: &RenderFrame) -> Result<(), TuiError> {
        match self {
            Self::Stdout { handle, .. } => {
                queue!(handle, MoveTo(0, 0), Clear(ClearType::All))?;
                for (row, line) in frame.lines().iter().enumerate() {
                    if row > 0 {
                        let row = u16::try_from(row).map_err(|_| {
                            TuiError::Render("frame exceeds terminal height".into())
                        })?;
                        queue!(handle, MoveTo(0, row))?;
                    }
                    write_line_stdout(handle, line)?;
                }
                handle.flush()?;
                Ok(())
            }
            Self::Buffer(buffer) => {
                buffer.clear();
                for (row, line) in frame.lines().iter().enumerate() {
                    if row > 0 {
                        buffer.extend_from_slice(b"\n");
                    }
                    write_line_buffer(buffer, line);
                }
                Ok(())
            }
        }
    }
}

impl Drop for TerminalTarget {
    fn drop(&mut self) {
        if let Self::Stdout {
            handle,
            raw_mode,
            alternate_screen,
            cursor_hidden,
        } = self
        {
            let stdout = handle;
            if *cursor_hidden {
                let _ = execute!(stdout, Show);
            }
            if *alternate_screen {
                let _ = execute!(stdout, LeaveAlternateScreen);
            }
            if *raw_mode {
                let _ = terminal::disable_raw_mode();
            }
        }
    }
}

fn write_line_stdout(handle: &mut Stdout, line: &RenderLine) -> Result<(), TuiError> {
    for segment in line.segments() {
        let styled = segment.as_styled_content();
        queue!(handle, PrintStyledContent(styled))?;
    }
    Ok(())
}

fn write_line_buffer(buffer: &mut Vec<u8>, line: &RenderLine) {
    for segment in line.segments() {
        buffer.extend_from_slice(segment.content().as_bytes());
    }
}

/// Thin wrapper around the concrete terminal output target.
#[derive(Debug)]
pub struct Terminal {
    target: TerminalTarget,
}

impl Terminal {
    /// Creates a terminal bound to the process `stdout` handle, enabling raw mode
    /// and entering the alternate screen buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the terminal cannot be initialised or if manipulating
    /// the TTY fails.
    pub fn stdout() -> Result<Self, TuiError> {
        Ok(Self {
            target: TerminalTarget::stdout()?,
        })
    }

    /// Creates a buffered terminal useful for tests.
    #[must_use]
    pub const fn buffered() -> Self {
        Self {
            target: TerminalTarget::buffered(),
        }
    }

    /// Returns the current terminal size.
    ///
    /// # Errors
    ///
    /// Returns an error when querying the terminal size fails.
    pub fn size(&self) -> Result<(u16, u16), TuiError> {
        let size = terminal::size()?;
        Ok(size)
    }

    /// Renders a frame to the terminal.
    ///
    /// # Errors
    ///
    /// Returns an error when the terminal cannot be updated.
    pub fn render(&mut self, frame: &RenderFrame) -> Result<(), TuiError> {
        self.target.write_frame(frame)
    }

    /// Returns the buffered contents when the terminal was created via [`Self::buffered`].
    #[must_use]
    pub const fn snapshot(&self) -> Option<&[u8]> {
        match &self.target {
            TerminalTarget::Buffer(buffer) => Some(buffer.as_slice()),
            TerminalTarget::Stdout { .. } => None,
        }
    }
}
