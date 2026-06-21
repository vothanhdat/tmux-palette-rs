//! Raw terminal handling via libc — the piece the TS version got for free from
//! Node's `tty` + `process` APIs.
//!
//! Notably this mirrors libuv's raw mode rather than `cfmakeraw`: input
//! processing/echo/canonical/signals are disabled, but output post-processing
//! (`OPOST | ONLCR`) is kept so the renderer's `\n` line separators still return
//! the cursor to column 0 — exactly what Node did, and what the renderer assumes.

use std::io;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};

static WINCH: AtomicBool = AtomicBool::new(false);
static EXIT: AtomicBool = AtomicBool::new(false);

extern "C" fn on_winch(_sig: libc::c_int) {
    WINCH.store(true, Ordering::SeqCst);
}

extern "C" fn on_term(_sig: libc::c_int) {
    EXIT.store(true, Ordering::SeqCst);
}

/// Was a SIGWINCH received since the last check? (clears the flag)
pub fn take_winch() -> bool {
    WINCH.swap(false, Ordering::SeqCst)
}

/// Was a terminating signal (SIGTERM/SIGHUP) received?
pub fn should_exit() -> bool {
    EXIT.load(Ordering::SeqCst)
}

fn install(sig: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handler as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        // No SA_RESTART: we want blocking read() to return EINTR so the loop can
        // react to resize / termination signals promptly.
        sa.sa_flags = 0;
        libc::sigaction(sig, &sa, std::ptr::null_mut());
    }
}

pub fn install_signal_handlers() {
    install(libc::SIGWINCH, on_winch);
    install(libc::SIGTERM, on_term);
    install(libc::SIGHUP, on_term);
}

pub struct RawMode {
    fd: RawFd,
    orig: libc::termios,
    active: bool,
}

impl RawMode {
    pub fn enable() -> io::Result<RawMode> {
        let fd = libc::STDIN_FILENO;
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut orig) != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut raw = orig;
            raw.c_iflag &= !(libc::IGNBRK
                | libc::BRKINT
                | libc::PARMRK
                | libc::ISTRIP
                | libc::INLCR
                | libc::IGNCR
                | libc::ICRNL
                | libc::IXON);
            // Keep output post-processing so '\n' is expanded to CR-LF.
            raw.c_oflag |= libc::OPOST | libc::ONLCR;
            raw.c_cflag &= !(libc::CSIZE | libc::PARENB);
            raw.c_cflag |= libc::CS8;
            raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(fd, libc::TCSANOW, &raw) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(RawMode {
                fd,
                orig,
                active: true,
            })
        }
    }

    /// Restore the original terminal attributes (idempotent).
    pub fn disable(&mut self) {
        if self.active {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &self.orig);
            }
            self.active = false;
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        self.disable();
    }
}

/// True when both stdin and stdout are TTYs (the palette requires it).
pub fn is_interactive() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 && libc::isatty(libc::STDOUT_FILENO) == 1 }
}

/// Terminal size as (columns, rows), defaulting to 80x24.
pub fn terminal_size() -> (i64, i64) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            (ws.ws_col as i64, ws.ws_row as i64)
        } else {
            (80, 24)
        }
    }
}

/// Blocking read of stdin bytes. Returns `Ok(0)` on EOF; surfaces EINTR as
/// `ErrorKind::Interrupted` so the caller can handle pending signals.
pub fn read_stdin(buf: &mut [u8]) -> io::Result<usize> {
    let n = unsafe {
        libc::read(
            libc::STDIN_FILENO,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
        )
    };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/// Write all bytes directly to stdout (fd 1), bypassing std's line-buffering so
/// each frame goes out as a single contiguous write (no flicker).
pub fn write_stdout(data: &[u8]) {
    let mut off = 0;
    while off < data.len() {
        let n = unsafe {
            libc::write(
                libc::STDOUT_FILENO,
                data[off..].as_ptr() as *const libc::c_void,
                data.len() - off,
            )
        };
        if n <= 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        off += n as usize;
    }
}
