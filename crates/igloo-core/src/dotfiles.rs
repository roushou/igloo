//! A person's dotfiles: the repository cloned into a new workspace and the command that
//! installs them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ValidationErrors, Validator};

/// The dotfiles a repository's new workspaces are set up with.
///
/// Invariants: `repository` is a non-empty URL of at most [`Dotfiles::MAX_REPOSITORY_BYTES`]
/// bytes starting with `https://`, `http://`, `ssh://` or `git@`, without whitespace or control
/// characters; `install` is non-blank, at most [`Dotfiles::MAX_INSTALL_BYTES`] bytes and
/// contains no NUL.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawDotfiles", into = "RawDotfiles")]
pub struct Dotfiles {
    repository: String,
    install: String,
}

#[derive(Serialize, Deserialize)]
struct RawDotfiles {
    repository: String,
    install: String,
}

impl Dotfiles {
    /// The longest repository URL accepted.
    pub const MAX_REPOSITORY_BYTES: usize = 2048;
    /// The longest install command accepted.
    pub const MAX_INSTALL_BYTES: usize = 4096;
    const SCHEMES: [&'static str; 4] = ["https://", "http://", "ssh://", "git@"];

    /// Dotfiles cloned from `repository` and installed by running `install` in the clone,
    /// reporting every invalid field.
    pub fn new(
        repository: impl Into<String>,
        install: impl Into<String>,
    ) -> Result<Self, ValidationErrors> {
        let repository = repository.into();
        let install = install.into();
        let (repository, install) = Validator::new()
            .field("repository", Self::check_repository(repository))
            .field("install", Self::check_install(install))
            .finish()?;
        Ok(Self {
            repository,
            install,
        })
    }

    /// The URL the dotfiles are cloned from.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// The shell command run in the clone to install them.
    #[must_use]
    pub fn install(&self) -> &str {
        &self.install
    }

    fn check_repository(repository: String) -> Result<String, String> {
        if repository.len() > Self::MAX_REPOSITORY_BYTES {
            return Err(format!(
                "must be at most {} bytes",
                Self::MAX_REPOSITORY_BYTES
            ));
        }
        if repository.contains(|c: char| c.is_whitespace() || c.is_control()) {
            return Err("must not contain whitespace or control characters".to_owned());
        }
        if !Self::SCHEMES
            .iter()
            .any(|scheme| repository.starts_with(scheme))
        {
            return Err("must start with https://, http://, ssh:// or git@".to_owned());
        }
        Ok(repository)
    }

    fn check_install(install: String) -> Result<String, String> {
        if install.trim().is_empty() {
            return Err("must not be empty".to_owned());
        }
        if install.len() > Self::MAX_INSTALL_BYTES {
            return Err(format!("must be at most {} bytes", Self::MAX_INSTALL_BYTES));
        }
        if install.contains('\0') {
            return Err("must not contain NUL".to_owned());
        }
        Ok(install)
    }
}

impl TryFrom<RawDotfiles> for Dotfiles {
    type Error = ValidationErrors;

    fn try_from(raw: RawDotfiles) -> Result<Self, Self::Error> {
        Self::new(raw.repository, raw.install)
    }
}

impl From<Dotfiles> for RawDotfiles {
    fn from(dotfiles: Dotfiles) -> Self {
        Self {
            repository: dotfiles.repository,
            install: dotfiles.install,
        }
    }
}

impl fmt::Debug for Dotfiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dotfiles")
            .field("repository", &self.repository)
            .field("install", &self.install)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_dotfiles_keep_their_parts() {
        for repository in [
            "https://github.com/me/dotfiles",
            "http://git.example/me/dotfiles.git",
            "ssh://git@example.com/me/dotfiles.git",
            "git@github.com:me/dotfiles.git",
        ] {
            let dotfiles = Dotfiles::new(repository, "./install.sh").expect(repository);
            assert_eq!(dotfiles.repository(), repository);
            assert_eq!(dotfiles.install(), "./install.sh");
        }
    }

    #[test]
    fn every_invalid_field_is_reported() {
        let errors = Dotfiles::new("--upload-pack=x", " ").expect_err("invalid");
        let fields: Vec<_> = errors.fields().map(|(field, _)| field).collect();
        assert_eq!(fields, ["install", "repository"]);
        for repository in [
            "",
            "github.com/me/dotfiles",
            "ext::sh -c id",
            "https://x y",
            "https://x\n",
        ] {
            assert!(Dotfiles::new(repository, "make").is_err(), "{repository:?}");
        }
        assert!(Dotfiles::new("https://x", "a\0b").is_err());
        assert!(Dotfiles::new("https://x", "a".repeat(Dotfiles::MAX_INSTALL_BYTES + 1)).is_err());
    }

    #[test]
    fn deserializing_validates() {
        let json = serde_json::json!({ "repository": "nope", "install": "make" });
        assert!(serde_json::from_value::<Dotfiles>(json).is_err());
        let dotfiles = Dotfiles::new("https://x", "make").expect("valid");
        let json = serde_json::to_value(&dotfiles).expect("json");
        assert_eq!(
            serde_json::from_value::<Dotfiles>(json).ok(),
            Some(dotfiles)
        );
    }
}
