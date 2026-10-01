//! Desktop notifications programs send through the terminal (OSC 9, 777 and 99).
//!
//! `alacritty_terminal` ignores these sequences, so the PTY's output is scanned for them on its
//! way to the parser (see `tap`). Agents use them to say a turn is done or that they need you.

/// A notification a program sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: Option<String>,
    pub body: String,
}

/// Longest sequence we keep. Anything longer isn't a notification worth showing.
const MAX_LEN: usize = 1024;
/// Kitty notifications can arrive in chunks; this many may be in flight at once.
const MAX_PENDING: usize = 8;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// Saw ESC.
    Esc,
    /// Inside `ESC ]`, keeping the bytes.
    Osc,
    /// Inside an OSC we don't care about (or one too long): wait for its end.
    Skip,
    /// Saw ESC inside an OSC: `\` ends it.
    OscEsc { skipping: bool },
}

/// Finds notification sequences in a byte stream, across reads.
#[derive(Debug, Default)]
pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
    /// Kitty chunks waiting for their last part: (id, title, body).
    pending: Vec<(String, String, String)>,
}

impl OscScanner {
    /// Scans the next bytes of output, calling `found` for every complete notification.
    pub fn feed(&mut self, bytes: &[u8], mut found: impl FnMut(Notification)) {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground {
                // Fast path: nothing to do until the next escape.
                match bytes[i..].iter().position(|&b| b == ESC) {
                    Some(at) => {
                        i += at + 1;
                        self.state = State::Esc;
                    }
                    None => return,
                }
                continue;
            }
            let b = bytes[i];
            i += 1;
            self.state = match self.state {
                State::Ground => unreachable!("handled above"),
                State::Esc => match b {
                    b']' => {
                        self.buf.clear();
                        State::Osc
                    }
                    ESC => State::Esc,
                    _ => State::Ground,
                },
                State::Osc => match b {
                    BEL => {
                        self.finish(&mut found);
                        State::Ground
                    }
                    ESC => State::OscEsc { skipping: false },
                    // CAN and SUB cancel a sequence.
                    0x18 | 0x1a => State::Ground,
                    _ if self.buf.len() >= MAX_LEN => State::Skip,
                    // The first `;` ends the OSC's number: only notifications are worth keeping.
                    b';' if self.buf.iter().all(u8::is_ascii_digit)
                        && !matches!(&self.buf[..], b"9" | b"99" | b"777") =>
                    {
                        State::Skip
                    }
                    _ => {
                        self.buf.push(b);
                        State::Osc
                    }
                },
                State::Skip => match b {
                    BEL | 0x18 | 0x1a => State::Ground,
                    ESC => State::OscEsc { skipping: true },
                    _ => State::Skip,
                },
                State::OscEsc { skipping } => match b {
                    b'\\' => {
                        if !skipping {
                            self.finish(&mut found);
                        }
                        State::Ground
                    }
                    // A new escape cut the sequence short; this byte starts it.
                    b']' => {
                        self.buf.clear();
                        State::Osc
                    }
                    ESC => State::Esc,
                    _ => State::Ground,
                },
            };
        }
    }

    fn finish(&mut self, found: &mut impl FnMut(Notification)) {
        let buf = std::mem::take(&mut self.buf);
        let Some((code, rest)) = split_once(&buf, b';') else { return };
        let rest = String::from_utf8_lossy(rest);
        let note = match code {
            b"9" => osc9(&rest),
            b"777" => osc777(&rest),
            b"99" => self.osc99(&rest),
            _ => None,
        };
        if let Some(note) = note {
            found(note);
        }
        // Keep the allocation for the next sequence.
        self.buf = buf;
    }

    /// Kitty: `99 ; key=value:key=value ; payload`. `p` says whether the payload is the title
    /// (the default) or the body, `d=0` that more chunks follow under the same `i`.
    fn osc99(&mut self, rest: &str) -> Option<Notification> {
        let (meta, payload) = rest.split_once(';')?;
        let (mut id, mut done, mut is_body) = ("", true, false);
        for pair in meta.split(':') {
            match pair.split_once('=') {
                Some(("i", v)) => id = v,
                Some(("d", v)) => done = v != "0",
                Some(("p", v)) => match v {
                    "title" => is_body = false,
                    "body" => is_body = true,
                    // Icons, buttons, queries: nothing to show.
                    _ => return None,
                },
                // Base64 payloads aren't worth decoding for a status line.
                Some(("e", "1")) => return None,
                _ => {}
            }
        }
        let index = match self.pending.iter().position(|(i, ..)| i == id) {
            Some(index) => index,
            None => {
                if self.pending.len() == MAX_PENDING {
                    self.pending.remove(0);
                }
                self.pending.push((id.to_owned(), String::new(), String::new()));
                self.pending.len() - 1
            }
        };
        let entry = &mut self.pending[index];
        let part = if is_body { &mut entry.2 } else { &mut entry.1 };
        if part.len() + payload.len() <= MAX_LEN {
            part.push_str(payload);
        }
        if !done {
            return None;
        }
        let (_, title, body) = self.pending.remove(index);
        notification(Some(&title), &body)
    }
}

/// iTerm2: `9 ; body`. ConEmu uses `9 ; <number> ; …` for other things (progress bars, cwd).
fn osc9(rest: &str) -> Option<Notification> {
    let (head, _) = rest.split_once(';').unwrap_or((rest, ""));
    if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    notification(None, rest)
}

/// urxvt and Ghostty: `777 ; notify ; title ; body`.
fn osc777(rest: &str) -> Option<Notification> {
    let rest = rest.strip_prefix("notify;")?;
    let (title, body) = rest.split_once(';').unwrap_or((rest, ""));
    notification(Some(title), body)
}

/// Cleans up the parts; a title alone becomes the body.
fn notification(title: Option<&str>, body: &str) -> Option<Notification> {
    let clean = |s: &str| -> String {
        let s: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        s.trim().to_owned()
    };
    let title = title.map(clean).filter(|t| !t.is_empty());
    let body = clean(body);
    match (title, body.is_empty()) {
        (None, true) => None,
        (Some(title), true) => Some(Notification { title: None, body: title }),
        (title, false) => Some(Notification { title, body }),
    }
}

fn split_once(bytes: &[u8], sep: u8) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|&b| b == sep)?;
    Some((&bytes[..at], &bytes[at + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<Notification> {
        let mut scanner = OscScanner::default();
        let mut out = Vec::new();
        for chunk in chunks {
            scanner.feed(chunk, |n| out.push(n));
        }
        out
    }

    fn body(body: &str) -> Notification {
        Notification { title: None, body: body.to_owned() }
    }

    #[test]
    fn osc9_with_either_terminator() {
        assert_eq!(
            scan(&[b"before\x1b]9;Claude needs your permission\x07after"]),
            [body("Claude needs your permission")]
        );
        assert_eq!(scan(&[b"\x1b]9;done\x1b\\"]), [body("done")]);
    }

    #[test]
    fn split_across_reads() {
        assert_eq!(scan(&[b"x\x1b", b"]9;hel", b"lo\x1b", b"\\y"]), [body("hello")]);
    }

    #[test]
    fn conemu_subcommands_and_other_oscs_are_ignored() {
        assert!(scan(&[b"\x1b]9;4;1;50\x07", b"\x1b]9;9;/tmp\x07"]).is_empty());
        assert!(scan(&[b"\x1b]0;a title\x07\x1b]8;;https://x\x07link\x1b]8;;\x07"]).is_empty());
        assert!(scan(&[b"\x1b]9;\x07"]).is_empty());
    }

    #[test]
    fn osc777() {
        assert_eq!(
            scan(&[b"\x1b]777;notify;Codex;Agent turn complete\x07"]),
            [Notification { title: Some("Codex".into()), body: "Agent turn complete".into() }]
        );
    }

    #[test]
    fn kitty_chunks_join_by_id() {
        assert_eq!(scan(&[b"\x1b]99;;Simple\x1b\\"]), [body("Simple")]);
        assert_eq!(
            scan(&[b"\x1b]99;i=7:d=0;Claude Code\x1b\\", b"\x1b]99;i=7:p=body;Waiting for input\x1b\\"]),
            [Notification { title: Some("Claude Code".into()), body: "Waiting for input".into() }]
        );
        assert!(scan(&[b"\x1b]99;e=1;aGk=\x1b\\"]).is_empty());
    }

    #[test]
    fn overlong_and_cancelled_sequences_are_dropped() {
        let mut long = b"\x1b]9;".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_LEN * 2));
        long.push(BEL);
        long.extend(b"\x1b]9;next\x07");
        assert_eq!(scan(&[&long]), [body("next")]);
        assert!(scan(&[b"\x1b]9;nope\x18\x07"]).is_empty());
    }

    #[test]
    fn an_escape_inside_restarts() {
        assert_eq!(scan(&[b"\x1b]9;cut\x1b]9;whole\x07"]), [body("whole")]);
        assert_eq!(scan(&[b"\x1b]9;a\x1b[0m\x1b]9;b\x07"]), [body("b")]);
    }

    #[test]
    fn control_characters_are_stripped() {
        assert_eq!(scan(&[b"\x1b]9;  two\tparts \x07"]), [body("two parts")]);
    }
}
