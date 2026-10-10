//! Git's credential helper for Igloo: answers with the API token for Igloo's own git endpoint
//! and for no other host.

use std::io::Read as _;
use std::process::ExitCode;

use clap::{Args, ValueEnum};
use igloo_git::RemoteUrl;

use crate::error::CliError;

/// Answers git's credential requests with the API token, so a clone of Igloo's git endpoint
/// needs no token on its URL. Configure it with
/// `git config credential.<api url>.helper '!igloo git-credential'`.
#[derive(Args)]
pub(crate) struct GitCredential {
    /// What git asks for.
    operation: Operation,
}

/// What git asks a credential helper to do.
#[derive(Clone, Copy, ValueEnum)]
enum Operation {
    /// Supplies a credential for a request.
    Get,
    /// Remembers one; Igloo's token is configured, never stored.
    Store,
    /// Forgets one; Igloo's token is configured, never stored.
    Erase,
}

/// The request git describes on the helper's input: `key=value` lines up to a blank line.
struct Request {
    protocol: Option<String>,
    host: Option<String>,
}

impl GitCredential {
    /// The user name git is given; Igloo reads the token from the password.
    const USER: &'static str = "igloo";

    /// Reads git's request from standard input and prints the answer.
    pub(crate) fn execute(self, api: &str, token: &str) -> Result<ExitCode, CliError> {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        print!("{}", self.answer(api, token, &input));
        Ok(ExitCode::SUCCESS)
    }

    /// What to print for `request`: the token when `get` asks for the host `api` names over
    /// its scheme and a token is set, nothing otherwise.
    fn answer(&self, api: &str, token: &str, request: &str) -> String {
        let Operation::Get = self.operation else {
            return String::new();
        };
        let request = Request::parse(request);
        let Ok(api) = RemoteUrl::try_from(format!("{}/", api.trim_end_matches('/'))) else {
            return String::new();
        };
        let scheme = api.as_str().split_once("://").map(|(scheme, _)| scheme);
        let same = request.protocol.as_deref() == scheme
            && request.host.is_some()
            && request.host.as_deref() == api.host();
        if same && !token.is_empty() {
            format!("username={}\npassword={token}\n", Self::USER)
        } else {
            String::new()
        }
    }
}

impl Request {
    fn parse(text: &str) -> Self {
        let mut request = Self {
            protocol: None,
            host: None,
        };
        for line in text.lines().take_while(|line| !line.is_empty()) {
            match line.split_once('=') {
                Some(("protocol", value)) => request.protocol = Some(value.to_owned()),
                Some(("host", value)) => request.host = Some(value.to_owned()),
                _ => {}
            }
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const API: &str = "http://igloo.example:7000";

    fn helper(operation: Operation) -> GitCredential {
        GitCredential { operation }
    }

    #[test]
    fn igloo_gets_the_token_and_other_hosts_get_nothing() {
        let get = helper(Operation::Get);
        let igloo = "protocol=http\nhost=igloo.example:7000\npath=git/repo_1.git\n\n";
        assert_eq!(
            get.answer(API, "secret", igloo),
            "username=igloo\npassword=secret\n"
        );
        for other in [
            "protocol=https\nhost=igloo.example:7000\n\n",
            "protocol=http\nhost=github.com\n\n",
            "protocol=http\nhost=igloo.example\n\n",
            "protocol=http\n\n",
            "",
        ] {
            assert_eq!(get.answer(API, "secret", other), "", "{other:?}");
        }
        assert_eq!(get.answer(API, "", igloo), "", "no token, no answer");
    }

    #[test]
    fn storing_and_erasing_answer_nothing() {
        let igloo = "protocol=http\nhost=igloo.example:7000\n\n";
        assert_eq!(helper(Operation::Store).answer(API, "secret", igloo), "");
        assert_eq!(helper(Operation::Erase).answer(API, "secret", igloo), "");
    }
}
