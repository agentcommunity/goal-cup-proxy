//! Terminal lifetime and input; no application actions run while this adapter owns input.

use super::{Menu, render, theme::Style};
use std::io::{self, Write};

pub fn interactive() -> bool {
    goalcup_platform::terminal::is_foreground(libc::STDIN_FILENO)
        && goalcup_platform::terminal::is_terminal(libc::STDOUT_FILENO)
        && std::env::var("TERM").is_ok_and(|term| !term.is_empty() && term != "dumb")
}

/// Return a zero-based selection, or cancellation. Arrow keys and j/k are supported.
/// A scoped terminal guard restores echo/canonical input before the caller performs setup.
pub fn select(menu: &Menu<'_>) -> io::Result<Option<usize>> {
    if !interactive() || menu.choices.is_empty() {
        return Ok(None);
    }
    let _guard = InputGuard::take()?;
    let color = std::env::var_os("NO_COLOR").is_none();
    let unicode = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        .is_some_and(|value| value.to_ascii_lowercase().contains("utf"));
    let animate = color && std::env::var_os("GOALCUP_NO_ANIMATION").is_none();
    let mut output = io::stdout().lock();
    let mut selected = 0;
    let mut phase = 0;
    let mut previous_rows = 0;
    let result = (|| {
        loop {
            let (width, height) = dimensions();
            if width < 28 || height < 9 {
                return Err(io::Error::other(
                    "enlarge the terminal to at least 28 columns and 9 rows, or use goalcup prep --harness <id>",
                ));
            }
            let style = Style {
                width,
                height,
                color,
                unicode,
                animate,
            };
            let frame = render(menu, selected, phase, style);
            if previous_rows > 0 {
                write!(output, "\r\x1b[{previous_rows}A")?;
            }
            // Clear only the frame reserved below the cursor; never erase scrollback.
            write!(output, "\x1b[J{}", frame.replace('\n', "\r\n"))?;
            output.flush()?;
            previous_rows = frame.lines().count().saturating_sub(1);
            if !ready(if animate { 180 } else { -1 })? {
                phase += 1;
                continue;
            }
            let Some(byte) = read_byte()? else {
                return Ok(None);
            };
            match byte {
                b'\r' | b'\n' => return Ok(Some(selected)),
                3 | 4 | 26 | b'q' => return Ok(None),
                b'j' => selected = (selected + 1) % menu.choices.len(),
                b'k' => selected = (selected + menu.choices.len() - 1) % menu.choices.len(),
                27 => {
                    // Escape alone cancels. Never block waiting for the rest of a key.
                    if !ready(60)? {
                        return Ok(None);
                    }
                    let Some(byte) = read_byte()? else {
                        return Ok(None);
                    };
                    if byte != b'[' && byte != b'O' {
                        continue;
                    }
                    if !ready(60)? {
                        continue;
                    }
                    match read_byte()? {
                        Some(b'A') => {
                            selected = (selected + menu.choices.len() - 1) % menu.choices.len()
                        }
                        Some(b'B') => selected = (selected + 1) % menu.choices.len(),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    })();
    writeln!(output, "\r")?;
    result
}

fn read_byte() -> io::Result<Option<u8>> {
    let mut byte = 0u8;
    // SAFETY: read writes at most one byte into a live local. Read without stdin's
    // user-space buffer, so poll also notices the remaining bytes of an arrow key.
    match unsafe { libc::read(libc::STDIN_FILENO, (&raw mut byte).cast(), 1) } {
        0 => Ok(None),
        1 => Ok(Some(byte)),
        _ => Err(io::Error::last_os_error()),
    }
}

fn dimensions() -> (usize, usize) {
    // SAFETY: ioctl writes only into the initialized winsize local.
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0
            && size.ws_col > 0
            && size.ws_row > 0
        {
            (usize::from(size.ws_col), usize::from(size.ws_row))
        } else {
            (80, 24)
        }
    }
}

fn ready(timeout: i32) -> io::Result<bool> {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one initialized pollfd, valid for the duration of the call.
    let result = unsafe { libc::poll(&mut fd, 1, timeout) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(result > 0)
}

struct InputGuard {
    // EchoOff saves the complete mode set and restores it both on drop and on terminating
    // signals. It is held only for this picker, before prep can ask for credentials.
    _saved: goalcup_platform::terminal::EchoOff,
}

impl InputGuard {
    fn take() -> io::Result<Self> {
        let saved = goalcup_platform::terminal::EchoOff::take(libc::STDIN_FILENO)
            .ok_or_else(io::Error::last_os_error)?;
        // SAFETY: tcgetattr initializes a local; tcsetattr reads that initialized local.
        unsafe {
            let mut modes: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut modes) != 0 {
                return Err(io::Error::last_os_error());
            }
            // Typed Ctrl-C is read as cancellation so the guard always restores the modes.
            modes.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
            modes.c_iflag &= !(libc::IXON | libc::ICRNL);
            modes.c_cc[libc::VMIN] = 1;
            modes.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &modes) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self { _saved: saved })
    }
}
