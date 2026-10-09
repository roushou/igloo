use std::collections::BTreeMap;

use igloo_core::ValidationErrors;
use igloo_core::process::{Argv, EnvVars};
use serde_json::Value;

use super::transcript::Entry;
use super::{HarnessKind, ToolSpec};

/// Adapts one coding tool: the process of each turn, and the reading of its output.
pub trait Harness: Send + Sync {
    /// The process of a turn asked `prompt`; `resume` when the tool's session from the task's
    /// earlier turns continues.
    fn turn(
        &self,
        tool: &ToolSpec,
        prompt: &str,
        resume: bool,
    ) -> Result<TurnCommand, ValidationErrors>;

    /// The transcript entries of one line of the tool's standard output.
    fn read(&self, line: &str) -> Vec<Entry>;

    /// Whether one line of the tool's output says it stopped on its account's usage limit,
    /// so the turn can be asked again once the limit resets.
    fn limited(&self, _line: &str) -> bool {
        false
    }
}

/// The process of one turn: a shell script with the prompt in `IGLOO_PROMPT`, so prompts never
/// need quoting. The task adds its git identity to the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnCommand {
    /// The program and its arguments.
    pub argv: Argv,
    /// Environment added for the turn.
    pub env: EnvVars,
}

/// Claude Code, headless, streaming JSON; later turns continue the sandbox's latest session.
pub struct ClaudeCode;

/// Codex, headless; its output is read line by line.
pub struct Codex;

/// Any command, given the prompt as its last argument; its output is read line by line.
pub struct CommandHarness;

impl TurnCommand {
    /// The variable holding the prompt.
    pub const PROMPT: &'static str = "IGLOO_PROMPT";

    /// Runs `script` with the prompt in [`TurnCommand::PROMPT`].
    pub fn new(script: String, prompt: &str) -> Result<Self, ValidationErrors> {
        let argv = Argv::try_from(vec!["sh".to_owned(), "-c".to_owned(), script])
            .map_err(|error| ValidationErrors::single("argv", error.to_string()))?;
        let env = EnvVars::try_from(BTreeMap::from([(
            Self::PROMPT.to_owned(),
            prompt.to_owned(),
        )]))?;
        Ok(Self { argv, env })
    }
}

impl HarnessKind {
    /// The harness of this kind.
    #[must_use]
    pub fn harness(self) -> &'static dyn Harness {
        match self {
            Self::ClaudeCode => &ClaudeCode,
            Self::Codex => &Codex,
            Self::Command => &CommandHarness,
        }
    }
}

impl Harness for ClaudeCode {
    fn turn(
        &self,
        _: &ToolSpec,
        prompt: &str,
        resume: bool,
    ) -> Result<TurnCommand, ValidationErrors> {
        let resume = if resume { " --continue" } else { "" };
        // Sandboxes run as root, where Claude Code skips permission prompts only when told it
        // runs in a sandbox.
        let script = format!(
            "IS_SANDBOX=1 claude -p \"${}\"{resume} --output-format stream-json --verbose \
             --dangerously-skip-permissions",
            TurnCommand::PROMPT
        );
        TurnCommand::new(script, prompt)
    }

    fn limited(&self, line: &str) -> bool {
        let line = line.to_ascii_lowercase();
        ["session limit", "usage limit", "weekly limit"]
            .iter()
            .any(|limit| line.contains(&format!("hit your {limit}")))
    }

    fn read(&self, line: &str) -> Vec<Entry> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Entry::output(line);
        };
        match message["type"].as_str() {
            Some("assistant") => Self::contents(&message)
                .iter()
                .filter_map(|content| match content["type"].as_str() {
                    Some("text") => Some(Entry::Message {
                        text: Self::string(&content["text"]),
                    }),
                    Some("tool_use") => Some(Entry::ToolCall {
                        id: Self::string(&content["id"]),
                        name: Self::string(&content["name"]),
                        input: content["input"].to_string(),
                    }),
                    _ => None,
                })
                .collect(),
            Some("user") => Self::contents(&message)
                .iter()
                .filter(|content| content["type"] == "tool_result")
                .map(|content| Entry::ToolResult {
                    id: Self::string(&content["tool_use_id"]),
                    output: Self::text(&content["content"]),
                    is_error: content["is_error"].as_bool().unwrap_or(false),
                })
                .collect(),
            Some("result") if message["is_error"].as_bool() == Some(true) => vec![Entry::Error {
                text: Self::string(&message["result"]),
            }],
            Some("system" | "result") => Vec::new(),
            _ => Entry::output(line),
        }
    }
}

impl ClaudeCode {
    fn contents(message: &Value) -> &[Value] {
        message["message"]["content"]
            .as_array()
            .map_or(&[], Vec::as_slice)
    }

    fn string(value: &Value) -> String {
        value.as_str().unwrap_or_default().to_owned()
    }

    /// A tool result's content: a string, or text blocks joined by newlines.
    fn text(value: &Value) -> String {
        match value {
            Value::String(text) => text.clone(),
            Value::Array(blocks) => blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    }
}

impl Harness for Codex {
    fn turn(&self, _: &ToolSpec, prompt: &str, _: bool) -> Result<TurnCommand, ValidationErrors> {
        TurnCommand::new(
            format!("codex exec --full-auto \"${}\"", TurnCommand::PROMPT),
            prompt,
        )
    }

    fn read(&self, line: &str) -> Vec<Entry> {
        Entry::output(line)
    }
}

impl Harness for CommandHarness {
    fn turn(
        &self,
        tool: &ToolSpec,
        prompt: &str,
        _: bool,
    ) -> Result<TurnCommand, ValidationErrors> {
        let command = tool.command.as_deref().unwrap_or("false");
        TurnCommand::new(format!("{command} \"${}\"", TurnCommand::PROMPT), prompt)
    }

    fn read(&self, line: &str) -> Vec<Entry> {
        Entry::output(line)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn tool(harness: HarnessKind) -> ToolSpec {
        ToolSpec {
            harness,
            install: None,
            command: Some("./agent --quiet".to_owned()),
            secrets: BTreeSet::new(),
            timeout_seconds: 60,
            max_turns: 1,
        }
    }

    fn turn(harness: HarnessKind, resume: bool) -> TurnCommand {
        harness
            .harness()
            .turn(&tool(harness), "it's \"quoted\"", resume)
            .expect("turn")
    }

    #[test]
    fn the_prompt_travels_in_the_environment() {
        let turn = turn(HarnessKind::Command, false);
        assert_eq!(turn.argv.program(), "sh");
        assert_eq!(
            turn.argv.args(),
            ["-c", "./agent --quiet \"$IGLOO_PROMPT\""]
        );
        assert_eq!(
            turn.env.iter().find(|(key, _)| *key == "IGLOO_PROMPT"),
            Some(("IGLOO_PROMPT", "it's \"quoted\""))
        );
    }

    #[test]
    fn claude_code_reports_its_usage_limit() {
        assert!(ClaudeCode.limited(
            r#"{"type":"result","is_error":true,"result":"You've hit your session limit · resets 8am (UTC)"}"#
        ));
        assert!(ClaudeCode.limited("You've hit your usage limit"));
        assert!(
            !ClaudeCode
                .limited(r#"{"type":"result","is_error":true,"result":"Reached max turns"}"#)
        );
        assert!(!CommandHarness.limited("You've hit your session limit"));
    }

    #[test]
    fn claude_code_continues_its_session_on_later_turns() {
        let first = turn(HarnessKind::ClaudeCode, false);
        assert!(
            first.argv.args()[1]
                .starts_with("IS_SANDBOX=1 claude -p \"$IGLOO_PROMPT\" --output-format")
        );
        let later = turn(HarnessKind::ClaudeCode, true);
        assert!(
            later.argv.args()[1]
                .starts_with("IS_SANDBOX=1 claude -p \"$IGLOO_PROMPT\" --continue ")
        );
    }

    #[test]
    fn recorded_claude_code_output_reads_as_messages_tool_calls_and_results() {
        let entries: Vec<Entry> = include_str!("fixtures/claude-code.jsonl")
            .lines()
            .flat_map(|line| ClaudeCode.read(line))
            .collect();
        assert_eq!(
            entries,
            [
                Entry::Message {
                    text: "I'll look at the failing test first.".to_owned()
                },
                Entry::ToolCall {
                    id: "toolu_01".to_owned(),
                    name: "Bash".to_owned(),
                    input: r#"{"command":"cargo test -p igloo-core","description":"Run the core tests"}"#
                        .to_owned()
                },
                Entry::ToolResult {
                    id: "toolu_01".to_owned(),
                    output: "test result: FAILED. 1 failed".to_owned(),
                    is_error: true
                },
                Entry::ToolCall {
                    id: "toolu_02".to_owned(),
                    name: "Edit".to_owned(),
                    input: r#"{"file_path":"src/lib.rs","new_string":"b","old_string":"a"}"#
                        .to_owned()
                },
                Entry::ToolResult {
                    id: "toolu_02".to_owned(),
                    output: "The file src/lib.rs has been updated.".to_owned(),
                    is_error: false
                },
                Entry::Message {
                    text: "Fixed: the test passes now.".to_owned()
                },
            ]
        );
    }

    #[test]
    fn unreadable_claude_code_lines_and_failed_results_are_kept() {
        assert_eq!(
            ClaudeCode.read("Error: not logged in"),
            [Entry::Output {
                text: "Error: not logged in".to_owned()
            }]
        );
        assert_eq!(
            ClaudeCode.read(
                r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"Reached max turns"}"#
            ),
            [Entry::Error {
                text: "Reached max turns".to_owned()
            }]
        );
    }
}
