use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use igloo_core::repo::SecretName;
use igloo_core::{Digest, ValidationErrors};
use serde::{Deserialize, Serialize};

/// A repository's coding tools, read from `.igloo/agents.toml` on its default branch. The JSON
/// Schema of the file is `schemas/agents.schema.json`.
///
/// Invariant: `default_tool`, when set, names one of `tools`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentSettings {
    default_tool: Option<ToolName>,
    tools: BTreeMap<ToolName, ToolSpec>,
}

/// The name of a coding tool in a repository's settings.
///
/// Invariant: 1 to 64 characters among `a-z`, `0-9` and `-`, starting with a letter.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ToolName(String);

/// The tool name is malformed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("must be 1 to 64 characters among a-z, 0-9 and -, starting with a letter")]
pub struct InvalidToolName;

/// How Igloo drives a coding tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessKind {
    /// Claude Code, headless, with streamed JSON output.
    ClaudeCode,
    /// Codex, headless.
    Codex,
    /// Any command; its raw output is the transcript.
    Command,
}

/// One coding tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// How Igloo drives it.
    pub harness: HarnessKind,
    /// The shell command installing it over the warm snapshot, once per agent snapshot.
    pub install: Option<String>,
    /// The command a `command` harness runs; the task's goal is its last argument.
    pub command: Option<String>,
    /// Repository secrets the tool gets, such as its credential.
    pub secrets: BTreeSet<SecretName>,
    /// How long one turn may run, in seconds.
    pub timeout_seconds: u32,
    /// How many turns a task may take.
    pub max_turns: u32,
}

/// The file as written. Keys read elsewhere, such as `protected`, are accepted.
#[derive(Deserialize)]
struct RawSettings {
    default_tool: Option<String>,
    #[serde(default)]
    tools: BTreeMap<String, RawTool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTool {
    harness: HarnessKind,
    install: Option<String>,
    command: Option<String>,
    #[serde(default)]
    secrets: Vec<String>,
    timeout_seconds: Option<u32>,
    max_turns: Option<u32>,
}

impl AgentSettings {
    /// Where the file lives in a repository.
    pub const PATH: &'static str = ".igloo/agents.toml";
    const DEFAULT_TIMEOUT: u32 = 3600;
    const MAX_TIMEOUT: u32 = 86_400;
    const DEFAULT_MAX_TURNS: u32 = 10;
    const MAX_TURNS: u32 = 100;

    /// The tool `name` names, or the default tool when `name` is `None`.
    #[must_use]
    pub fn tool(&self, name: Option<&ToolName>) -> Option<(&ToolName, &ToolSpec)> {
        let name = name.or(self.default_tool.as_ref())?;
        self.tools.get_key_value(name)
    }

    /// The default tool's name, if the repository sets one.
    #[must_use]
    pub const fn default_tool(&self) -> Option<&ToolName> {
        self.default_tool.as_ref()
    }

    fn tool_spec(raw: RawTool, field: &str, errors: &mut ValidationErrors) -> ToolSpec {
        let timeout_seconds = raw.timeout_seconds.unwrap_or(Self::DEFAULT_TIMEOUT);
        if !(1..=Self::MAX_TIMEOUT).contains(&timeout_seconds) {
            errors.add(
                format!("{field}.timeout_seconds"),
                "must be between 1 and 86400 seconds",
            );
        }
        let max_turns = raw.max_turns.unwrap_or(Self::DEFAULT_MAX_TURNS);
        if !(1..=Self::MAX_TURNS).contains(&max_turns) {
            errors.add(format!("{field}.max_turns"), "must be between 1 and 100");
        }
        if raw.harness == HarnessKind::Command && raw.command.is_none() {
            errors.add(
                format!("{field}.command"),
                "a command harness needs a command",
            );
        }
        let mut secrets = BTreeSet::new();
        for (index, name) in raw.secrets.iter().enumerate() {
            match name.parse::<SecretName>() {
                Ok(name) => {
                    secrets.insert(name);
                }
                Err(error) => errors.add(format!("{field}.secrets.{index}"), error.to_string()),
            }
        }
        ToolSpec {
            harness: raw.harness,
            install: raw.install,
            command: raw.command,
            secrets,
            timeout_seconds,
            max_turns,
        }
    }
}

impl TryFrom<&str> for AgentSettings {
    type Error = ValidationErrors;

    /// Parses and validates the file, reporting every problem.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let raw: RawSettings = toml::from_str(text)
            .map_err(|error| ValidationErrors::single("agents", error.message().to_owned()))?;
        let mut errors = ValidationErrors::default();
        let mut tools = BTreeMap::new();
        for (name, tool) in raw.tools {
            let field = format!("tools.{name}");
            let Ok(parsed) = name.parse::<ToolName>() else {
                errors.add(field, InvalidToolName.to_string());
                continue;
            };
            tools.insert(parsed, Self::tool_spec(tool, &field, &mut errors));
        }
        let default_tool = match raw.default_tool.map(|name| name.parse::<ToolName>()) {
            None => None,
            Some(Ok(name)) if tools.contains_key(&name) => Some(name),
            Some(_) => {
                errors.add("default_tool", "must name one of tools");
                None
            }
        };
        errors.into_result(Self {
            default_tool,
            tools,
        })
    }
}

impl ToolSpec {
    /// The key of the agent snapshot this tool's `install` builds over the snapshot keyed
    /// `over`, such as a warm snapshot's key. Equal inputs give equal keys.
    #[must_use]
    pub fn snapshot_key(&self, over: &Digest) -> Digest {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"igloo.agent.v1\n");
        hasher.update(format!("over {over}\n").as_bytes());
        if let Some(install) = &self.install {
            hasher.update(format!("install {install}\n").as_bytes());
        }
        Digest::from_blake3(*hasher.finalize().as_bytes())
    }
}

impl ToolName {
    /// The name as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ToolName {
    type Err = InvalidToolName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();
        let valid = s.len() <= 64
            && chars.next().is_some_and(|first| first.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(InvalidToolName)
        }
    }
}

impl TryFrom<String> for ToolName {
    type Error = InvalidToolName;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        name.parse()
    }
}

impl From<ToolName> for String {
    fn from(name: ToolName) -> Self {
        name.0
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(name: &str) -> ToolName {
        name.parse().expect("name")
    }

    #[test]
    fn igloos_own_settings_name_claude_code_as_the_default_tool() {
        let settings =
            AgentSettings::try_from(include_str!("../../../../.igloo/agents.toml")).expect("valid");
        let (tool, spec) = settings.tool(None).expect("default tool");
        assert_eq!(tool.as_str(), "claude-code");
        assert_eq!(spec.harness, HarnessKind::ClaudeCode);
        assert!(spec.install.is_some());
        assert!(
            spec.secrets
                .contains(&"CLAUDE_CODE_OAUTH_TOKEN".parse().expect("secret"))
        );
    }

    #[test]
    fn a_repository_without_tools_has_no_default() {
        let settings = AgentSettings::try_from("protected = [\"a\"]").expect("valid");
        assert_eq!(settings, AgentSettings::default());
        assert_eq!(settings.tool(None), None);
    }

    #[test]
    fn a_task_may_pick_another_tool_and_defaults_apply() {
        let settings = AgentSettings::try_from(
            "default_tool = \"a\"\n[tools.a]\nharness = \"claude-code\"\n\
             [tools.b]\nharness = \"command\"\ncommand = \"./agent\"\n",
        )
        .expect("valid");
        let (tool, spec) = settings.tool(Some(&name("b"))).expect("b");
        assert_eq!(tool, &name("b"));
        assert_eq!(spec.command.as_deref(), Some("./agent"));
        assert_eq!(spec.timeout_seconds, 3600);
        assert_eq!(spec.max_turns, 10);
        assert_eq!(settings.tool(Some(&name("c"))), None);
    }

    #[test]
    fn every_problem_is_reported_with_its_field() {
        let errors = AgentSettings::try_from(
            "default_tool = \"missing\"\n[tools.Bad]\nharness = \"command\"\n\
             [tools.ok]\nharness = \"command\"\nsecrets = [\"no spaces\"]\n\
             timeout_seconds = 0\nmax_turns = 101\n",
        )
        .expect_err("invalid");
        let fields: Vec<&str> = errors.fields().map(|(field, _)| field).collect();
        assert_eq!(
            fields,
            [
                "default_tool",
                "tools.Bad",
                "tools.ok.command",
                "tools.ok.max_turns",
                "tools.ok.secrets.0",
                "tools.ok.timeout_seconds",
            ]
        );
    }

    #[test]
    fn an_agent_snapshot_key_changes_with_what_it_goes_over_and_its_install() {
        let spec = |install: &str| ToolSpec {
            harness: HarnessKind::ClaudeCode,
            install: Some(install.to_owned()),
            command: None,
            secrets: BTreeSet::new(),
            timeout_seconds: 60,
            max_turns: 1,
        };
        let warm = Digest::from_blake3([1; 32]);
        let key = spec("install a").snapshot_key(&warm);
        assert_eq!(key, spec("install a").snapshot_key(&warm));
        assert_ne!(key, spec("install b").snapshot_key(&warm));
        assert_ne!(
            key,
            spec("install a").snapshot_key(&Digest::from_blake3([2; 32]))
        );
    }

    #[test]
    fn the_json_schema_names_every_field() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../../schemas/agents.schema.json"))
                .expect("schema");
        let keys = |value: &serde_json::Value| -> Vec<String> {
            let mut keys: Vec<String> = value
                .as_object()
                .expect("properties")
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        };
        let properties = &schema["properties"];
        assert_eq!(keys(properties), ["default_tool", "protected", "tools"]);
        assert_eq!(
            keys(&properties["tools"]["additionalProperties"]["properties"]),
            [
                "command",
                "harness",
                "install",
                "max_turns",
                "secrets",
                "timeout_seconds"
            ]
        );
    }
}
