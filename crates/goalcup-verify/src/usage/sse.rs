//! A plain incremental SSE framer over decoded text (PRD-09 section 2): byte pieces in, each
//! event's joined `data:` payload out at a blank line. Comments, keep-alives, `event:` and `id:`
//! lines are skipped. A stray CR before LF is dropped.

/// No `Debug`: it holds body bytes.
#[derive(Default)]
pub struct Sse {
    line: Vec<u8>,
    data: Vec<u8>,
    has: bool,
}

impl Sse {
    /// Feeds the next piece; `out` gets each finished event's payload.
    pub fn feed(&mut self, bytes: &[u8], out: &mut impl FnMut(&[u8])) {
        for &b in bytes {
            if b == b'\n' {
                self.line_end(out);
            } else {
                self.line.push(b);
            }
        }
    }

    /// Flushes a last event that had no trailing blank line.
    pub fn finish(&mut self, out: &mut impl FnMut(&[u8])) {
        if !self.line.is_empty() {
            self.line_end(out);
        }
        self.line_end(out);
    }

    /// Drops a half-read line and event, after a hole in the stream.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn line_end(&mut self, out: &mut impl FnMut(&[u8])) {
        let mut line = std::mem::take(&mut self.line);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.is_empty() {
            if std::mem::take(&mut self.has) {
                out(&self.data);
            }
            self.data.clear();
        } else if let Some(rest) = line.strip_prefix(b"data") {
            let v = match rest {
                [] => &[][..],
                [b':', b' ', v @ ..] | [b':', v @ ..] => v,
                _ => return,
            };
            if self.has {
                self.data.push(b'\n');
            }
            self.data.extend_from_slice(v);
            self.has = true;
        }
    }
}
