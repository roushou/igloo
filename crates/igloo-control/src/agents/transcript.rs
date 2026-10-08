use std::sync::Arc;

use igloo_core::job::JobId;
use igloo_core::process::OutputStream;

use super::task::Task;
use crate::ports::{LogStore, StorageError};

/// One thing a tool did or said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Text the tool wrote to the reader.
    Message {
        /// The text.
        text: String,
    },
    /// The tool called one of its tools.
    ToolCall {
        /// The call's id, matching its result.
        id: String,
        /// The tool called, such as `Bash`.
        name: String,
        /// Its input, as JSON.
        input: String,
    },
    /// What a tool call returned.
    ToolResult {
        /// The call's id.
        id: String,
        /// The output.
        output: String,
        /// Whether the call failed.
        is_error: bool,
    },
    /// A line of output the harness does not read further.
    Output {
        /// The line.
        text: String,
    },
    /// The tool reported a failure.
    Error {
        /// What it said.
        text: String,
    },
}

/// An entry of a task's transcript, with the turn it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptEntry {
    /// The turn, from 1.
    pub turn: u32,
    /// The entry.
    pub entry: Entry,
}

/// Reads tasks' transcripts from their turns' output, as stored: secrets are already masked.
/// Entries come from complete lines only, so a transcript only grows while a turn runs.
#[derive(Clone)]
pub struct Transcripts {
    logs: Arc<dyn LogStore>,
}

impl Entry {
    /// `line` as one output entry.
    #[must_use]
    pub fn output(line: &str) -> Vec<Self> {
        vec![Self::Output {
            text: line.to_owned(),
        }]
    }
}

impl Transcripts {
    const PAGE: usize = 256;

    /// Transcripts read from `logs`.
    #[must_use]
    pub fn new(logs: Arc<dyn LogStore>) -> Self {
        Self { logs }
    }

    /// `task`'s transcript, oldest first: its tool's standard output read by its harness, and
    /// its standard error as output lines.
    pub async fn of(&self, task: &Task) -> Result<Vec<TranscriptEntry>, StorageError> {
        let Some(prepared) = task.settings() else {
            return Ok(Vec::new());
        };
        let harness = prepared.spec.harness.harness();
        let mut entries = Vec::new();
        for (turn, number) in task.turns().iter().zip(1..) {
            for (stream, line) in self.lines(turn.job, turn.ending.is_some()).await? {
                let read = match stream {
                    OutputStream::Stdout => harness.read(&line),
                    OutputStream::Stderr => Entry::output(&line),
                };
                entries.extend(read.into_iter().map(|entry| TranscriptEntry {
                    turn: number,
                    entry,
                }));
            }
        }
        Ok(entries)
    }

    /// The complete lines of `job`'s output in the order written; the unterminated last line
    /// of each stream too once `ended`.
    async fn lines(
        &self,
        job: JobId,
        ended: bool,
    ) -> Result<Vec<(OutputStream, String)>, StorageError> {
        let mut lines = Lines::default();
        let mut complete = Vec::new();
        let mut after = 0;
        loop {
            let page = self.logs.read(job, after, Self::PAGE).await?;
            let Some(last) = page.last() else {
                break;
            };
            after = last.sequence;
            for entry in &page {
                complete.extend(lines.push(entry.stream, &entry.data));
            }
            if page.len() < Self::PAGE {
                break;
            }
        }
        if ended {
            complete.extend(lines.finish());
        }
        Ok(complete)
    }
}

/// Splits each stream into lines across chunks.
#[derive(Default)]
struct Lines {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Lines {
    /// The lines `data` completes on `stream`.
    fn push(&mut self, stream: OutputStream, data: &[u8]) -> Vec<(OutputStream, String)> {
        let buffer = self.buffer(stream);
        buffer.extend_from_slice(data);
        let mut complete = Vec::new();
        while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=end).collect();
            let text = String::from_utf8_lossy(line.get(..end).unwrap_or_default())
                .trim_end_matches('\r')
                .to_owned();
            if !text.is_empty() {
                complete.push((stream, text));
            }
        }
        complete
    }

    /// The unterminated last line of each stream.
    fn finish(&mut self) -> Vec<(OutputStream, String)> {
        [OutputStream::Stdout, OutputStream::Stderr]
            .into_iter()
            .filter_map(|stream| {
                let rest = std::mem::take(self.buffer(stream));
                let text = String::from_utf8_lossy(&rest).trim().to_owned();
                (!text.is_empty()).then_some((stream, text))
            })
            .collect()
    }

    const fn buffer(&mut self, stream: OutputStream) -> &mut Vec<u8> {
        match stream {
            OutputStream::Stdout => &mut self.stdout,
            OutputStream::Stderr => &mut self.stderr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_span_chunks_and_the_last_one_waits_for_the_end() {
        let mut lines = Lines::default();
        assert_eq!(lines.push(OutputStream::Stdout, b"fir"), []);
        assert_eq!(
            lines.push(OutputStream::Stderr, b"oops\n"),
            [(OutputStream::Stderr, "oops".to_owned())]
        );
        assert_eq!(
            lines.push(OutputStream::Stdout, b"st\r\n\nsecond\nthi"),
            [
                (OutputStream::Stdout, "first".to_owned()),
                (OutputStream::Stdout, "second".to_owned())
            ]
        );
        assert_eq!(lines.finish(), [(OutputStream::Stdout, "thi".to_owned())]);
    }
}
