use globset::{GlobSet, GlobSetBuilder};
use igloo_core::ValidationErrors;
use serde::Deserialize;

/// A repository's trust settings, read from `.igloo/agents.toml` on its target branch: never
/// from the change being judged.
///
/// Invariant: `protected` matches exactly the paths its patterns name.
#[derive(Clone, Debug)]
pub struct TrustSettings {
    patterns: Vec<String>,
    protected: GlobSet,
}

/// The file as written. Keys this phase does not read yet are accepted.
#[derive(Deserialize)]
struct RawTrust {
    #[serde(default)]
    protected: Vec<String>,
}

impl TrustSettings {
    /// Where the file lives in a repository.
    pub const PATH: &'static str = ".igloo/agents.toml";

    /// The protected path patterns, as written.
    #[must_use]
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// The paths among `paths` a protected pattern matches, in order.
    #[must_use]
    pub fn protected<'a>(&self, paths: &'a [String]) -> Vec<&'a str> {
        paths
            .iter()
            .filter(|path| self.protected.is_match(path.as_str()))
            .map(String::as_str)
            .collect()
    }
}

impl Default for TrustSettings {
    /// Nothing protected, as for a repository without the file.
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            protected: GlobSet::empty(),
        }
    }
}

impl TryFrom<&str> for TrustSettings {
    type Error = ValidationErrors;

    /// Parses the file: `protected` lists glob patterns of repository-relative paths, where `*`
    /// stays within one directory and `**` crosses directories.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let raw: RawTrust = toml::from_str(text)
            .map_err(|error| ValidationErrors::single("agents", error.message().to_owned()))?;
        let mut builder = GlobSetBuilder::new();
        let mut errors = ValidationErrors::default();
        for (index, pattern) in raw.protected.iter().enumerate() {
            match globset::GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
            {
                Ok(glob) => {
                    builder.add(glob);
                }
                Err(error) => errors.add(format!("protected.{index}"), error.kind().to_string()),
            }
        }
        errors.into_result(())?;
        let protected = builder
            .build()
            .map_err(|error| ValidationErrors::single("protected", error.to_string()))?;
        Ok(Self {
            patterns: raw.protected,
            protected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn igloos_protected_paths_match_its_contract_surfaces() {
        let settings =
            TrustSettings::try_from(include_str!("../../../../.igloo/agents.toml")).expect("valid");
        let paths: Vec<String> = [
            "crates/igloo-control/src/ports/forge.rs",
            "crates/igloo-control/migrations/1_init.sql",
            "proto/igloo/worker/v1/worker.proto",
            "AGENTS.md",
            "crates/igloo-core/AGENTS.md",
            ".igloo/pipeline.toml",
            "crates/igloo-control/src/platform/change.rs",
            "README.md",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(
            settings.protected(&paths),
            [
                "crates/igloo-control/src/ports/forge.rs",
                "crates/igloo-control/migrations/1_init.sql",
                "proto/igloo/worker/v1/worker.proto",
                "AGENTS.md",
                "crates/igloo-core/AGENTS.md",
                ".igloo/pipeline.toml",
            ]
        );
    }

    #[test]
    fn a_single_star_stays_in_its_directory_and_bad_patterns_are_reported() {
        let settings = TrustSettings::try_from("protected = [\"docs/*.md\"]").expect("valid");
        let paths = ["docs/a.md".to_owned(), "docs/adr/b.md".to_owned()];
        assert_eq!(settings.protected(&paths), ["docs/a.md"]);
        let errors = TrustSettings::try_from("protected = [\"[\"]").expect_err("invalid");
        assert_eq!(
            errors.fields().next().map(|(field, _)| field),
            Some("protected.0")
        );
    }
}
