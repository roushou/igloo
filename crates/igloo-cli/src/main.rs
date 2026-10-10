//! The Igloo CLI, installed as `igloo`.
//!
//! Uses `igloo-rs` for every API call and never constructs HTTP requests itself.

// A CLI's job is to talk to the terminal; this is the only crate allowed to print.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use clap::Parser;
use igloo::{Client, Error};

use commands::Command;
use commands::git_credential::GitCredential;
use error::CliError;

mod commands;
mod error;
mod git;
mod json;
mod label;

/// The server's REST API when neither `--api` nor `IGLOO_API` is given.
const DEFAULT_API: &str = "http://127.0.0.1:7000";

#[derive(Parser)]
#[command(
    name = "igloo",
    version,
    about = "Run and manage work on an Igloo server"
)]
struct Cli {
    /// The server's REST API.
    #[arg(
        long,
        env = "IGLOO_API",
        default_value = DEFAULT_API,
        global = true
    )]
    api: String,
    /// The API token.
    #[arg(long, env = "IGLOO_TOKEN", hide_env_values = true, global = true)]
    token: Option<String>,
    #[command(subcommand)]
    command: Invocation,
}

/// What the CLI is asked to do.
#[derive(clap::Subcommand)]
enum Invocation {
    /// Answers git's credential requests for Igloo's git endpoint with the API token.
    GitCredential(GitCredential),
    #[command(flatten)]
    Api(Command),
}

impl Cli {
    async fn execute(self) -> Result<ExitCode, CliError> {
        let token = self.token.unwrap_or_default();
        match self.command {
            Invocation::GitCredential(helper) => helper.execute(&self.api, &token),
            Invocation::Api(command) => {
                let client = Client::new(&self.api, token)?;
                command.execute(&client).await
            }
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match Cli::parse().execute().await {
        Ok(code) => code,
        Err(CliError::Api(Error::Api(problem))) => {
            eprintln!("error: {} ({})", problem.title, problem.code);
            for field in &problem.errors {
                eprintln!("  {}: {}", field.field, field.message);
            }
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("error: {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            ExitCode::from(2)
        }
    }
}
