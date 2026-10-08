# Igloo on Igloo

Igloo checks and merges its own changes like any repository's. GitHub hosts the git history and
GitHub Actions keeps building every commit independently.

## Once

1. Run the server and a Linux worker with the OCI runtime and overlays: `docs/deploy.md` on a
   host, or `docs/dev-linux.md` for development.
2. Create a fine-grained GitHub token for `roushou/igloo` with **Contents: read and write**. If
   `main` has branch protection or a ruleset, let the token's owner bypass it: Igloo pushes `main`
   when it merges.
3. Register the repository and store the token:

   ```sh
   export IGLOO_TOKEN=<dev token>
   igloo repo add github.com/roushou/igloo --token-secret GITHUB_TOKEN
   igloo secret set <repo id> GITHUB_TOKEN    # paste the token, then Ctrl-D
   ```

The first run imports the pipeline's image and builds the warm snapshot; later runs reuse it until
`Cargo.lock` or `rust-toolchain.toml` changes.

## For every change

```sh
git switch -c my-change
# edit, commit
git push origin my-change
igloo change create              # repository, branch and title come from the checkout
igloo change show <id>           # revisions, runs and checks; `igloo logs <job>` for output
git push origin my-change && igloo change push <id>   # after more commits: a new revision
igloo change approve <id>        # needed when protected paths changed (.igloo/agents.toml)
igloo change merge <id>          # Igloo pushes main
```

A merge is refused while the latest revision's checks have not passed, while protected paths lack
an approval of that revision, and when `main` moved: rebase, push and `igloo change push <id>`.

Every merge records an outcome event (`igloo.outcome.recorded`); a later `git revert` of its
commits on `main` is recorded against it (`igloo.outcome.reverted`).

## Agents

`.igloo/agents.toml` names Claude Code as the default tool. Once, store its subscription token:

```sh
claude setup-token                                  # prints the token
igloo secret set <repo id> CLAUDE_CODE_OAUTH_TOKEN  # paste it, then Ctrl-D
```

Add Igloo to the editor's agent (`docs/mcp.md`), then ask it to create tasks, for example
"create an Igloo task to implement P5.1 of docs/specs/phase-5.md". Each task starts from the head
of `main` in its own sandbox; the first one builds the agent snapshot (the warm snapshot with
Claude Code installed), later ones reuse it.

```sh
igloo task list                    # or task.list over MCP
igloo task transcript <id> --follow
igloo change show <change id>      # the task's change: revisions, runs, comments
igloo change comment <change id> "Handle the empty case" --path src/lib.rs --line 12
igloo change request-changes <change id>   # the agent takes another turn
igloo change approve <change id>           # protected paths need it
igloo change merge <change id>             # Igloo pushes main; the task is done
```

The task's outcome names the task, the tool and its turns (`igloo.outcome.attributed`).
