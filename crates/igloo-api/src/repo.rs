//! Repositories and their secrets.

use igloo_core::Entity;
use igloo_core::dotfiles::Dotfiles;
use igloo_core::repo::{BranchName, CommitId, Repo, RepoLocation, RepoValueError, SecretName};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{ValidationErrors, Validator};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Registers a repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct RegisterRepoRequest {
    /// `github.com/<owner>/<name>`, or an absolute path on the server for development.
    pub location: String,
    /// The branch changes merge into; `main` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// The repository secret holding the forge token, such as `GITHUB_TOKEN`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_secret: Option<String>,
}

/// A repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct RepoResource {
    /// Its id (`repo_...`).
    pub id: String,
    /// Where it is hosted.
    pub location: String,
    /// The branch changes merge into.
    pub default_branch: String,
    /// The secret holding the forge token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_secret: Option<String>,
    /// Where Igloo serves the repository over git, relative to the API: `/git/<id>.git`. Clone
    /// and push there with the API token as the password.
    #[serde(default)]
    pub git_path: String,
    /// The dotfiles new workspaces are set up with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dotfiles: Option<DotfilesResource>,
}

/// The dotfiles new workspaces of a repository are set up with: the repository is cloned and
/// the install command run in the clone, once, when a workspace is created.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct DotfilesResource {
    /// The URL to clone: `https://`, `http://`, `ssh://` or `git@`.
    pub repository: String,
    /// The shell command run in the clone; a failure is reported and does not fail the
    /// workspace.
    pub install: String,
}

impl DotfilesResource {
    /// Dotfiles cloned from `repository` and installed by `install`.
    #[must_use]
    pub fn new(repository: impl Into<String>, install: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            install: install.into(),
        }
    }
}

impl TryFrom<DotfilesResource> for Dotfiles {
    type Error = ValidationErrors;

    fn try_from(resource: DotfilesResource) -> Result<Self, Self::Error> {
        Self::new(resource.repository, resource.install)
    }
}

impl From<&Dotfiles> for DotfilesResource {
    fn from(dotfiles: &Dotfiles) -> Self {
        Self {
            repository: dotfiles.repository().to_owned(),
            install: dotfiles.install().to_owned(),
        }
    }
}

/// Makes a snapshot of a repository at a commit: its checkout under `/workspace`, with a shallow
/// `.git`, over `base`. Names exactly one of `branch` (fetched first) and `commit`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct RepoSnapshotRequest {
    /// A branch on the forge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// A commit of the default branch's history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The snapshot to check out over, such as an imported image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

/// A repository snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct RepoSnapshotResource {
    /// The snapshot.
    pub snapshot: String,
    /// The commit checked out.
    pub commit: String,
}

impl RepoSnapshotResource {
    /// Snapshot `snapshot` of `commit`.
    #[must_use]
    pub const fn new(snapshot: String, commit: String) -> Self {
        Self { snapshot, commit }
    }
}

/// What a validated [`RepoSnapshotRequest`] checks out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckoutTarget {
    /// The head of a branch.
    Branch(BranchName),
    /// A commit.
    Commit(CommitId),
}

impl RepoSnapshotRequest {
    /// A request for `branch`'s head.
    #[must_use]
    pub fn branch(branch: impl Into<String>) -> Self {
        Self {
            branch: Some(branch.into()),
            commit: None,
            base: None,
        }
    }

    /// A request for `commit`.
    #[must_use]
    pub fn commit(commit: impl Into<String>) -> Self {
        Self {
            branch: None,
            commit: Some(commit.into()),
            base: None,
        }
    }

    /// Sets the base snapshot.
    #[must_use]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }

    /// What to check out and over which snapshot, reporting every invalid field.
    pub fn target(&self) -> Result<(CheckoutTarget, Option<SnapshotId>), ValidationErrors> {
        let target = match (&self.branch, &self.commit) {
            (Some(branch), None) => branch
                .parse()
                .map(CheckoutTarget::Branch)
                .map_err(|error: RepoValueError| error.to_string()),
            (None, Some(commit)) => commit
                .parse()
                .map(CheckoutTarget::Commit)
                .map_err(|error: RepoValueError| error.to_string()),
            _ => Err("name exactly one of branch and commit".to_owned()),
        };
        Validator::new()
            .field("target", target)
            .field(
                "base",
                self.base
                    .as_deref()
                    .map(str::parse::<SnapshotId>)
                    .transpose(),
            )
            .finish()
    }
}

/// The names of a repository's secrets. Values are never returned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct SecretList {
    /// The names, sorted.
    pub names: Vec<String>,
}

/// A validated [`RegisterRepoRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRegistration {
    /// Where it is hosted.
    pub location: RepoLocation,
    /// The branch changes merge into.
    pub default_branch: BranchName,
    /// The secret holding the forge token.
    pub token: Option<SecretName>,
}

impl RegisterRepoRequest {
    /// A request for the repository at `location`, merging into `main`, without a token.
    #[must_use]
    pub fn new(location: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            default_branch: None,
            token_secret: None,
        }
    }

    /// Sets the default branch.
    #[must_use]
    pub fn with_default_branch(mut self, branch: impl Into<String>) -> Self {
        self.default_branch = Some(branch.into());
        self
    }

    /// Sets the secret holding the forge token.
    #[must_use]
    pub fn with_token_secret(mut self, name: impl Into<String>) -> Self {
        self.token_secret = Some(name.into());
        self
    }
}

impl TryFrom<RegisterRepoRequest> for RepoRegistration {
    type Error = ValidationErrors;

    fn try_from(request: RegisterRepoRequest) -> Result<Self, Self::Error> {
        let (location, default_branch, token) = Validator::new()
            .field("location", request.location.parse::<RepoLocation>())
            .field(
                "default_branch",
                request
                    .default_branch
                    .as_deref()
                    .unwrap_or("main")
                    .parse::<BranchName>(),
            )
            .field(
                "token_secret",
                request
                    .token_secret
                    .as_deref()
                    .map(str::parse::<SecretName>)
                    .transpose(),
            )
            .finish()?;
        Ok(Self {
            location,
            default_branch,
            token,
        })
    }
}

impl From<&Repo> for RepoResource {
    fn from(repo: &Repo) -> Self {
        Self {
            id: repo.id().to_string(),
            location: repo.location().to_string(),
            default_branch: repo.default_branch().to_string(),
            token_secret: repo.token().map(ToString::to_string),
            git_path: format!("/git/{}.git", repo.id()),
            dotfiles: repo.dotfiles().map(DotfilesResource::from),
        }
    }
}

impl From<Vec<SecretName>> for SecretList {
    fn from(names: Vec<SecretName>) -> Self {
        Self {
            names: names.into_iter().map(String::from).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotfiles_convert_both_ways_and_invalid_ones_report_their_fields() {
        let resource = DotfilesResource::new("https://github.com/me/dotfiles", "./install.sh");
        let dotfiles = Dotfiles::try_from(resource.clone()).expect("valid");
        assert_eq!(DotfilesResource::from(&dotfiles), resource);

        let errors = Dotfiles::try_from(DotfilesResource::new("nope", "")).expect_err("invalid");
        let fields: Vec<_> = errors.fields().map(|(field, _)| field).collect();
        assert_eq!(fields, ["install", "repository"]);
    }
}
