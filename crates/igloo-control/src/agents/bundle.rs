/// A turn's commits as they leave the sandbox: a job commits whatever the tool left
/// uncommitted, then writes a git bundle of every commit since the task's commit, base64
/// encoded, to its standard output. Nothing is written when there are no commits.
pub struct CommitBundle;

/// The output is not base64.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the bundle is not valid base64")]
pub struct InvalidBundle;

impl CommitBundle {
    /// The git identity of commits made in a task's sandbox, as environment variables.
    pub const IDENTITY: [(&'static str, &'static str); 4] = [
        ("GIT_AUTHOR_NAME", "Igloo agent"),
        ("GIT_AUTHOR_EMAIL", "agent@igloo.invalid"),
        ("GIT_COMMITTER_NAME", "Igloo agent"),
        ("GIT_COMMITTER_EMAIL", "agent@igloo.invalid"),
    ];

    /// The variable holding the task's commit.
    pub const BASE: &'static str = "IGLOO_BASE";
    /// The variable holding the message of the commit of uncommitted work.
    pub const MESSAGE: &'static str = "IGLOO_MESSAGE";

    /// The shell script of the job, run in the sandbox's working directory.
    pub const SCRIPT: &'static str = "set -e\n\
        if [ -n \"$(git status --porcelain)\" ]; then\n\
        \x20 git add --all && git commit --quiet --message \"$IGLOO_MESSAGE\"\n\
        fi\n\
        if [ \"$(git rev-parse HEAD)\" != \"$IGLOO_BASE\" ]; then\n\
        \x20 git bundle create --quiet .git/igloo.bundle \"$IGLOO_BASE..HEAD\"\n\
        \x20 base64 < .git/igloo.bundle\n\
        \x20 rm .git/igloo.bundle\n\
        fi\n";

    /// The bundle in the job's standard output, or `None` when the turn made no commits.
    pub fn decode(output: &[u8]) -> Result<Option<Vec<u8>>, InvalidBundle> {
        let sextets: Vec<u8> = output
            .iter()
            .filter(|byte| !byte.is_ascii_whitespace() && **byte != b'=')
            .map(|byte| Self::sextet(*byte))
            .collect::<Result<_, _>>()?;
        if sextets.is_empty() {
            return Ok(None);
        }
        if sextets.len() % 4 == 1 {
            return Err(InvalidBundle);
        }
        let mut bytes = Vec::with_capacity(sextets.len() * 3 / 4);
        for chunk in sextets.chunks(4) {
            let block = chunk
                .iter()
                .enumerate()
                .fold(0u32, |block, (index, sextet)| {
                    block | (u32::from(*sextet) << (18 - 6 * index))
                });
            let [_, a, b, c] = block.to_be_bytes();
            bytes.extend([a, b, c].into_iter().take(chunk.len() - 1));
        }
        Ok(Some(bytes))
    }

    fn sextet(byte: u8) -> Result<u8, InvalidBundle> {
        match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(InvalidBundle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_base64_decodes_and_no_output_means_no_commits() {
        assert_eq!(CommitBundle::decode(b""), Ok(None));
        assert_eq!(CommitBundle::decode(b" \n"), Ok(None));
        assert_eq!(
            CommitBundle::decode(b"aGVsbG8g\nd29ybGQ=\n"),
            Ok(Some(b"hello world".to_vec()))
        );
        assert_eq!(CommitBundle::decode(b"YQ=="), Ok(Some(b"a".to_vec())));
        assert_eq!(CommitBundle::decode(b"YWI="), Ok(Some(b"ab".to_vec())));
        assert_eq!(CommitBundle::decode(b"not base64!"), Err(InvalidBundle));
        assert_eq!(CommitBundle::decode(b"YWJjZ"), Err(InvalidBundle));
    }
}
