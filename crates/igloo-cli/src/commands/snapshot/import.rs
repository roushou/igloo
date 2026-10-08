//! Has the server pull a public container image and register it as a snapshot.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::snapshot::ImportImageRequest;

use crate::error::CliError;
use crate::json::Json;

/// Has the server pull a public container image and register it as a snapshot.
#[derive(Args)]
pub(crate) struct Import {
    /// The image, such as `rust:1.99` or `ghcr.io/owner/image:tag`.
    image: String,
    /// The `os/architecture` variant; the server's when unset.
    #[arg(long)]
    platform: Option<String>,
}

impl Import {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let mut request = ImportImageRequest::new(self.image);
        if let Some(platform) = self.platform {
            request = request.with_platform(platform);
        }
        println!("{}", Json::from(&client.import_image(&request).await?));
        Ok(ExitCode::SUCCESS)
    }
}
