use igloo_api::job::JobResource;

use crate::client::Error;

/// Which output stream a chunk came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// One event of a job's log stream.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LogEvent {
    /// Output from the job.
    Output {
        /// Where it was written.
        stream: OutputStream,
        /// The text, lossily decoded as UTF-8.
        data: String,
    },
    /// The job ended; this is the last event.
    End(Box<JobResource>),
}

/// A job's output as it arrives, read from the server-sent event stream.
#[derive(Debug)]
pub struct LogStream {
    response: reqwest::Response,
    buffer: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    ended: bool,
}

impl LogStream {
    pub(crate) const fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            buffer: Vec::new(),
            event: None,
            data: Vec::new(),
            ended: false,
        }
    }

    /// The next event, or `None` once the job ended and the stream closed.
    pub async fn next(&mut self) -> Option<Result<LogEvent, Error>> {
        loop {
            if self.ended {
                return None;
            }
            if let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim_end_matches(['\n', '\r']);
                if line.is_empty() {
                    if let Some(event) = self.dispatch() {
                        return Some(event);
                    }
                } else {
                    self.field(line);
                }
                continue;
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) => self.buffer.extend_from_slice(&chunk),
                Ok(None) => {
                    self.ended = true;
                    return None;
                }
                Err(error) => {
                    self.ended = true;
                    return Some(Err(error.into()));
                }
            }
        }
    }

    /// Records one `field: value` line of the current event.
    fn field(&mut self, line: &str) {
        let (name, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match name {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
    }

    /// Turns the accumulated fields into an event; `None` for events the SDK ignores.
    fn dispatch(&mut self) -> Option<Result<LogEvent, Error>> {
        let event = self.event.take();
        let data = std::mem::take(&mut self.data).join("\n");
        let stream = match event.as_deref() {
            Some("stdout") => OutputStream::Stdout,
            Some("stderr") => OutputStream::Stderr,
            Some("end") => {
                self.ended = true;
                return Some(
                    serde_json::from_str(&data)
                        .map(|job| LogEvent::End(Box::new(job)))
                        .map_err(|error| Error::Protocol(error.to_string())),
                );
            }
            Some("error") => {
                self.ended = true;
                return Some(Err(Error::Protocol(data)));
            }
            _ => return None,
        };
        Some(Ok(LogEvent::Output { stream, data }))
    }
}
