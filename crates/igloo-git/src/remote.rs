use std::fmt;
use std::str::FromStr;

use igloo_core::repo::{RepoLocation, RepoValueError};

use crate::error::GitValueError;

/// The name of a configured remote, such as `origin`.
///
/// Invariant: 1 to 100 bytes of ASCII alphanumerics, `.`, `_` and `-`, not starting with `-`
/// or `.`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RemoteName(String);

/// A URL git fetches from and pushes to, kept as written.
///
/// Invariant: an `https://`, `http://`, `ssh://` or `file://` URL, a scp-like
/// `[user@]host:path`, or an absolute path; with a host unless local; without control
/// characters; never starting with `-`. Other transports, such as `ext::`, are refused.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RemoteUrl {
    raw: String,
    scheme: Scheme,
    host: Option<String>,
    path: String,
}

/// How a remote is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// HTTP over TLS.
    Https,
    /// Plain HTTP.
    Http,
    /// SSH, as `ssh://` or scp-like.
    Ssh,
    /// The local file system.
    File,
}

/// HTTP credentials for a remote, sent as an `Authorization` header.
///
/// Invariant: never printed; `Debug` shows nothing of the secret.
#[derive(Clone)]
pub struct Credentials {
    authorization: String,
}

impl RemoteName {
    /// `origin`.
    pub fn origin() -> Self {
        Self("origin".to_owned())
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RemoteName {
    type Error = GitValueError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        let valid = !name.is_empty()
            && name.len() <= 100
            && !name.starts_with(['-', '.'])
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if valid {
            Ok(Self(name))
        } else {
            Err(GitValueError::RemoteName)
        }
    }
}

impl FromStr for RemoteName {
    type Err = GitValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl fmt::Display for RemoteName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl RemoteUrl {
    /// How the remote is reached.
    pub const fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The host, with its port if one is given; `None` for local remotes.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// The path on the host, as written.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The URL as written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// `host` without user information.
    fn host_of(authority: &str) -> Option<String> {
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        (!host.is_empty()).then(|| host.to_owned())
    }
}

impl TryFrom<String> for RemoteUrl {
    type Error = GitValueError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        if raw.is_empty() || raw.starts_with('-') || raw.contains(char::is_control) {
            return Err(GitValueError::RemoteUrl);
        }
        let (scheme, host, path) = if let Some((scheme, rest)) = raw.split_once("://") {
            let scheme = match scheme {
                "https" => Scheme::Https,
                "http" => Scheme::Http,
                "ssh" | "git+ssh" | "ssh+git" => Scheme::Ssh,
                "file" => Scheme::File,
                _ => return Err(GitValueError::RemoteUrl),
            };
            if scheme == Scheme::File {
                if !rest.starts_with('/') {
                    return Err(GitValueError::RemoteUrl);
                }
                (scheme, None, rest.to_owned())
            } else {
                let (authority, path) = rest.split_once('/').ok_or(GitValueError::RemoteUrl)?;
                let host = Self::host_of(authority).ok_or(GitValueError::RemoteUrl)?;
                (scheme, Some(host), format!("/{path}"))
            }
        } else if raw.starts_with('/') {
            (Scheme::File, None, raw.clone())
        } else {
            let (authority, path) = raw.split_once(':').ok_or(GitValueError::RemoteUrl)?;
            // `<transport>::<address>` names a remote helper, which is never allowed.
            if authority.contains('/') || path.is_empty() || path.starts_with(':') {
                return Err(GitValueError::RemoteUrl);
            }
            let host = Self::host_of(authority).ok_or(GitValueError::RemoteUrl)?;
            (Scheme::Ssh, Some(host), path.to_owned())
        };
        Ok(Self {
            raw,
            scheme,
            host,
            path,
        })
    }
}

impl FromStr for RemoteUrl {
    type Err = GitValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl fmt::Display for RemoteUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl From<&RepoLocation> for RemoteUrl {
    /// GitHub over HTTPS, or the local path.
    fn from(location: &RepoLocation) -> Self {
        match location {
            RepoLocation::Github { owner, name } => Self {
                raw: format!("https://github.com/{owner}/{name}.git"),
                scheme: Scheme::Https,
                host: Some("github.com".to_owned()),
                path: format!("/{owner}/{name}.git"),
            },
            RepoLocation::Local { path } => Self {
                raw: path.clone(),
                scheme: Scheme::File,
                host: None,
                path: path.clone(),
            },
        }
    }
}

impl TryFrom<&RemoteUrl> for RepoLocation {
    type Error = RepoValueError;

    /// A GitHub repository over any scheme, or a local absolute path.
    fn try_from(url: &RemoteUrl) -> Result<Self, Self::Error> {
        match url.host() {
            None => url.path().parse(),
            Some(host) if host.eq_ignore_ascii_case("github.com") => {
                format!("github.com/{}", url.path().trim_matches('/')).parse()
            }
            Some(_) => Err(RepoValueError::Location),
        }
    }
}

impl Credentials {
    /// HTTP basic authentication as `user` with `password`.
    pub fn basic(user: &str, password: &str) -> Self {
        let encoded = Self::base64(format!("{user}:{password}").as_bytes());
        Self {
            authorization: format!("Authorization: Basic {encoded}"),
        }
    }

    /// The `http.extraHeader` value carrying the credentials.
    pub(crate) fn header(&self) -> &str {
        &self.authorization
    }

    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let block = chunk.iter().enumerate().fold(0u32, |block, (index, byte)| {
                block | (u32::from(*byte) << (16 - 8 * index))
            });
            for index in 0..4 {
                if index <= chunk.len() {
                    let sextet = (block >> (18 - 6 * index)) & 0x3f;
                    encoded.push(char::from(ALPHABET[sextet as usize]));
                } else {
                    encoded.push('=');
                }
            }
        }
        encoded
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_parsed_by_transport() {
        let cases = [
            (
                "https://github.com/o/n.git",
                Scheme::Https,
                Some("github.com"),
                "/o/n.git",
            ),
            (
                "https://user@github.com/o/n",
                Scheme::Https,
                Some("github.com"),
                "/o/n",
            ),
            (
                "http://localhost:8080/r.git",
                Scheme::Http,
                Some("localhost:8080"),
                "/r.git",
            ),
            (
                "ssh://git@github.com/o/n.git",
                Scheme::Ssh,
                Some("github.com"),
                "/o/n.git",
            ),
            (
                "git@github.com:o/n.git",
                Scheme::Ssh,
                Some("github.com"),
                "o/n.git",
            ),
            ("file:///srv/r.git", Scheme::File, None, "/srv/r.git"),
            ("/srv/r.git", Scheme::File, None, "/srv/r.git"),
        ];
        for (raw, scheme, host, path) in cases {
            let url: RemoteUrl = raw.parse().unwrap();
            assert_eq!(
                (url.scheme(), url.host(), url.path()),
                (scheme, host, path),
                "{raw}"
            );
            assert_eq!(url.to_string(), raw);
        }
        for invalid in [
            "",
            "-uhttps://x/y",
            "ext::sh -c touch% /tmp/pwned",
            "relative/path",
            "file://host/path",
            "https://",
            "https:///path",
            "host:",
            "https://h/p\n",
        ] {
            assert!(invalid.parse::<RemoteUrl>().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn github_urls_name_repo_locations() {
        let github = RepoLocation::Github {
            owner: "roushou".to_owned(),
            name: "igloo".to_owned(),
        };
        for raw in [
            "https://github.com/roushou/igloo.git",
            "https://github.com/roushou/igloo",
            "git@github.com:roushou/igloo.git",
            "ssh://git@github.com/roushou/igloo/",
        ] {
            let url: RemoteUrl = raw.parse().unwrap();
            assert_eq!(RepoLocation::try_from(&url), Ok(github.clone()), "{raw}");
        }
        let local: RemoteUrl = "file:///srv/r.git".parse().unwrap();
        assert_eq!(
            RepoLocation::try_from(&local),
            Ok(RepoLocation::Local {
                path: "/srv/r.git".to_owned()
            })
        );
        let elsewhere: RemoteUrl = "https://gitlab.com/o/n.git".parse().unwrap();
        assert!(RepoLocation::try_from(&elsewhere).is_err());
        assert_eq!(
            RemoteUrl::from(&github).as_str(),
            "https://github.com/roushou/igloo.git"
        );
    }

    #[test]
    fn credentials_are_base64_encoded_and_never_shown() {
        let credentials = Credentials::basic("x-access-token", "secret");
        assert_eq!(
            credentials.header(),
            "Authorization: Basic eC1hY2Nlc3MtdG9rZW46c2VjcmV0"
        );
        assert_eq!(format!("{credentials:?}"), "Credentials(..)");
        assert_eq!(
            Credentials::basic("a", "").header(),
            "Authorization: Basic YTo="
        );
        assert_eq!(
            Credentials::basic("ab", "c").header(),
            "Authorization: Basic YWI6Yw=="
        );
    }
}
