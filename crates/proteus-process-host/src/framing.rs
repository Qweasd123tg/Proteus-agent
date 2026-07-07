use std::io::{BufRead, Write};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

/// Default per-frame safety limit for stdout parsing.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// A synchronous byte-stream framing for JSON messages.
pub trait Framing: Clone + Send + 'static {
    fn write_frame<W: Write>(&self, writer: &mut W, message: &Value) -> Result<()>;
    fn read_frame<R: BufRead>(&self, reader: &mut R) -> Result<Value>;
}

/// One compact JSON value per `\n`-terminated line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewlineJsonFraming {
    max_frame_bytes: usize,
}

impl NewlineJsonFraming {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self { max_frame_bytes }
    }

    pub fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }
}

impl Default for NewlineJsonFraming {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FRAME_BYTES)
    }
}

impl Framing for NewlineJsonFraming {
    fn write_frame<W: Write>(&self, writer: &mut W, message: &Value) -> Result<()> {
        writer.write_all(message.to_string().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn read_frame<R: BufRead>(&self, reader: &mut R) -> Result<Value> {
        let mut line = Vec::with_capacity(self.max_frame_bytes.min(8192));
        loop {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                if line.is_empty() {
                    bail!("child stdout closed before a frame was received");
                }
                break;
            }

            let bytes_to_take = buffer
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(buffer.len(), |position| position + 1);
            if line.len().saturating_add(bytes_to_take) > self.max_frame_bytes {
                bail!(
                    "newline JSON frame exceeded {} bytes before newline",
                    self.max_frame_bytes
                );
            }

            line.extend_from_slice(&buffer[..bytes_to_take]);
            reader.consume(bytes_to_take);

            if line.last() == Some(&b'\n') {
                break;
            }
        }

        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }

        let line = std::str::from_utf8(&line)?;
        serde_json::from_str(line).map_err(Into::into)
    }
}

/// LSP-style `Content-Length: N\r\n\r\n` framing with a JSON body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentLengthFraming {
    max_frame_bytes: usize,
}

impl ContentLengthFraming {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self { max_frame_bytes }
    }

    pub fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }
}

impl Default for ContentLengthFraming {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FRAME_BYTES)
    }
}

impl Framing for ContentLengthFraming {
    fn write_frame<W: Write>(&self, writer: &mut W, message: &Value) -> Result<()> {
        let body = message.to_string();
        write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
        writer.write_all(body.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    fn read_frame<R: BufRead>(&self, reader: &mut R) -> Result<Value> {
        let content_length = read_content_length(reader, self.max_frame_bytes)?;
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body)?;
        serde_json::from_slice(&body).map_err(Into::into)
    }
}

fn read_content_length<R: BufRead>(reader: &mut R, max_frame_bytes: usize) -> Result<usize> {
    let mut content_length = None;
    let mut header_bytes = 0usize;

    loop {
        let mut line = Vec::new();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            bail!("child stdout closed before content-length headers were received");
        }

        header_bytes = header_bytes.saturating_add(read);
        if header_bytes > 64 * 1024 {
            bail!("content-length headers exceeded 65536 bytes");
        }

        if line.ends_with(b"\n") {
            line.pop();
        }
        if line.ends_with(b"\r") {
            line.pop();
        }

        if line.is_empty() {
            let Some(length) = content_length else {
                bail!("content-length frame missing Content-Length header");
            };
            if length > max_frame_bytes {
                bail!("content-length frame exceeded {max_frame_bytes} bytes");
            }
            return Ok(length);
        }

        let line = std::str::from_utf8(&line)?;
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("Content-Length") {
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|error| anyhow!("invalid Content-Length header: {error}"))?;
            content_length = Some(length);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use super::*;

    /// Строка длиннее лимита без завершающего `\n` отклоняется до конца
    /// чтения — защита от неограниченного буфера.
    #[test]
    fn newline_framing_rejects_oversized_frame_without_newline() {
        let framing = NewlineJsonFraming::new(20_000);
        let payload = vec![b' '; 20_001];
        let mut reader = BufReader::new(&payload[..]);

        let error = framing
            .read_frame(&mut reader)
            .expect_err("oversized frame should fail");

        assert!(
            error
                .to_string()
                .contains("newline JSON frame exceeded 20000 bytes before newline"),
            "{error}"
        );
    }

    /// Увеличенный per-host лимит принимает кадр, который дефолтный
    /// лимит всё ещё отклоняет.
    #[test]
    fn newline_framing_honors_custom_limit() {
        let payload = "x".repeat(20_001);
        let frame = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"{payload}\"}}\n");

        let generous = NewlineJsonFraming::new(100_000);
        let mut reader = BufReader::new(frame.as_bytes());
        let value = generous
            .read_frame(&mut reader)
            .expect("custom limit should accept larger frame");
        assert_eq!(value["id"], 1);

        let strict = NewlineJsonFraming::new(20_000);
        let mut reader = BufReader::new(frame.as_bytes());
        strict
            .read_frame(&mut reader)
            .expect_err("strict limit should reject the same frame");
    }
}
