//! The PTY, with its output looked at on the way to the parser: for notifications (see `notify`)
//! and for how busy the terminal is (see `activity`).

use std::fs::File;
use std::io::{self, Read};
use std::sync::Arc;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use polling::{Event, PollMode, Poller};

use crate::activity;
use crate::events::{Shared, TermEvent};
use crate::notify::OscScanner;

pub(crate) struct TappedPty {
    pty: Pty,
    tap: Tap,
}

impl TappedPty {
    pub fn new(pty: Pty, shared: Arc<Shared>) -> io::Result<Self> {
        // A second handle on the same PTY: the event loop polls the original, reads through this.
        let file = pty.file().try_clone()?;
        Ok(Self { pty, tap: Tap { file, scanner: OscScanner::default(), shared } })
    }
}

pub(crate) struct Tap {
    file: File,
    scanner: OscScanner,
    shared: Arc<Shared>,
}

impl Read for Tap {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read(buf)?;
        if read > 0 {
            activity::output(&self.shared);
            let sink = &self.shared.sink;
            self.scanner.feed(&buf[..read], |n| sink(TermEvent::Notify { title: n.title, body: n.body }));
        }
        Ok(read)
    }
}

impl EventedReadWrite for TappedPty {
    type Reader = Tap;
    type Writer = File;

    // The trait declares this unsafe (registered sources must outlive their registration); we
    // only forward to the PTY, which keeps that promise, and the PTY lives as long as we do.
    #[allow(unsafe_code)]
    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        unsafe { self.pty.register(poll, interest, mode) }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        self.pty.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.pty.deregister(poll)
    }

    fn reader(&mut self) -> &mut Tap {
        &mut self.tap
    }

    fn writer(&mut self) -> &mut File {
        self.pty.writer()
    }
}

impl EventedPty for TappedPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

impl OnResize for TappedPty {
    fn on_resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size);
    }
}
