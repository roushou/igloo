use std::sync::Arc;

use igloo_core::job::Job;
use igloo_core::repo::SecretName;
use igloo_core::{Resource, ValidationErrors};

use crate::app::AppError;
use crate::platform::SandboxQueries;
use crate::ports::{SecretStore, SecretValue};

/// The secrets jobs are given: resolved from their sandbox's repository when leased, and
/// masked out of their output.
#[derive(Clone)]
pub(super) struct JobSecrets {
    sandboxes: SandboxQueries,
    store: Arc<dyn SecretStore>,
}

impl JobSecrets {
    const MASK: &'static [u8] = b"***";

    pub(super) fn new(sandboxes: SandboxQueries, store: Arc<dyn SecretStore>) -> Self {
        Self { sandboxes, store }
    }

    /// The values of the secrets `job` names, from its sandbox's repository; an error when the
    /// sandbox has no repository or a secret is not set.
    pub(super) async fn resolve(
        &self,
        job: &Job,
    ) -> Result<Vec<(SecretName, SecretValue)>, AppError> {
        let names = job.spec().secrets();
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let sandbox = self.sandboxes.get(job.spec().sandbox()).await?;
        let Some(repo) = sandbox.and_then(|sandbox| sandbox.spec().repo()) else {
            return Err(
                ValidationErrors::single("secrets", "the sandbox has no repository").into(),
            );
        };
        let mut resolved = Vec::with_capacity(names.len());
        for name in names {
            let value = self.store.get(repo, name).await?.ok_or_else(|| {
                AppError::from(ValidationErrors::single(
                    format!("secrets.{name}"),
                    "not set",
                ))
            })?;
            resolved.push((name.clone(), value));
        }
        Ok(resolved)
    }

    /// `data` with every occurrence of a non-empty value in `values` replaced by `***`.
    pub(super) fn mask(data: &[u8], values: &[SecretValue]) -> Vec<u8> {
        let mut masked = data.to_vec();
        for value in values {
            let needle = value.expose().as_bytes();
            if needle.is_empty() {
                continue;
            }
            let mut out = Vec::with_capacity(masked.len());
            let mut rest = masked.as_slice();
            while let Some(index) = rest
                .windows(needle.len())
                .position(|window| window == needle)
            {
                out.extend_from_slice(&rest[..index]);
                out.extend_from_slice(Self::MASK);
                rest = &rest[index + needle.len()..];
            }
            out.extend_from_slice(rest);
            masked = out;
        }
        masked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_occurrence_is_masked() {
        let token = SecretValue::try_from("hunter2".to_owned()).expect("value");
        let empty = SecretValue::try_from(String::new()).expect("value");
        assert_eq!(
            JobSecrets::mask(b"a hunter2 b hunter2", &[token, empty]),
            b"a *** b ***"
        );
    }
}
