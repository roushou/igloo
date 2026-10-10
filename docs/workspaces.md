# Workspaces

A workspace is your development environment on Igloo (ADR 0013): a long-lived sandbox on a branch
of a repository, forked from the repository's warm snapshot, with the network allowed and the
repository's secrets granted. You reach it with `igloo shell` from any machine and run your
editor and coding tools inside it. Nothing on your laptop needs to be kept in sync.

## Open one

```sh
export IGLOO_API=<api url> IGLOO_TOKEN=<token>
igloo workspace create --repo <repo id> --branch main   # prints the workspace; it starts at once
igloo shell <workspace id>                              # a terminal in it
```

`igloo shell` with no argument opens your most recently used workspace of the repository whose
registered location is the `origin` remote of the current directory (`--repo` names it instead).
It starts a stopped workspace and waits for it, puts your terminal in raw mode so every key
including Ctrl-C reaches the remote shell, follows your terminal's resizes, and exits with the
remote shell's exit code. If the connection is lost it exits with 255.

```sh
igloo workspace list              # your workspaces and their phase
igloo workspace stop <id>         # seals the sandbox's changes, then stops it
igloo workspace start <id>        # resumes from what the last stop sealed
igloo workspace delete <id>       # drops the changes
```

A workspace with no terminal attached for two hours stops itself. Stopping keeps the whole file
system of the sandbox (the checkout, `$HOME`, installed tools and editor plugins) and loses
processes: a `tmux` session does not survive a stop.

The sandbox is made from the repository's pipeline: its image, limits and environment
(`.igloo/pipeline.toml`), with the network allowed whatever the pipeline says. The checkout is in
`/workspace`, on the branch you opened, with `origin` pointing at Igloo. Tools your work needs
(git above all) must be in the image or the warm snapshot.

## Git from the workspace

The checkout's `origin` is Igloo's git endpoint for the repository, `<api url>/git/<repo id>.git`
(`IGLOO_PUBLIC_URL` decides the URL), so inside the workspace:

```sh
git switch -c my-change
# edit, commit
git push origin my-change     # opens a change, or revises the branch's open change
igloo change show <id>        # from anywhere
```

A push to the default branch is refused; it moves only by `igloo change merge`.

Terminals of a workspace carry a git credential of their own in the environment
(`GIT_CONFIG_*`, one `http.<api url>/git/.extraHeader`), not your API token, which never enters a
sandbox. The credential reaches only the workspace's repository, acts as you, and stops working
when the workspace is deleted or its sandbox ends. It lasts seven days and every terminal gets a
fresh one, so reopen a terminal if a session outlives it. It is a bearer token: anything running
in your sandbox can use it, as it can use any secret granted to the workspace.

Repository secrets named by the pipeline's `secrets` are also in the environment of your
terminals, with their current values. This is how a coding tool gets its API key:

```sh
igloo secret set <repo id> ANTHROPIC_API_KEY     # then list it under secrets in pipeline.toml
igloo shell
claude                                            # or any other coding tool
```

## Dotfiles

A repository can name your dotfiles and the command that installs them. Igloo clones them into
`~/.dotfiles` and runs the command there, once, when a workspace is created; stopping and
starting does not run it again, and changing the setting does not touch existing workspaces.

```sh
igloo repo dotfiles set https://github.com/me/dotfiles './install.sh' --repo <repo id>
igloo repo dotfiles clear --repo <repo id>
```

The repository URL starts with `https://`, `http://`, `ssh://` or `git@`, and the sandbox must be
able to clone it (a private repository needs credentials the sandbox has). The install command
is a shell command. A failing install does not fail the workspace: the setup job fails instead,
the server logs `workspace setup ... see the job's logs`, and the output is in the checkout at
`.git/igloo-setup.log`. The same job points `origin` at Igloo; it runs once per workspace.

On the development worker (`IGLOO_WORKER_RUNTIME=process`) nothing is isolated: the setup job
runs on the host, in the host's `$HOME`. Use it with dotfiles only on a throwaway host.

## Neovim and other editors

The terminal carries your editor. `TERM` is `xterm-256color`. Put the editor in the repository's
image, or install it with your dotfiles' install command; `~/.local` and your plugins are kept
across stops. For example, an install command that fetches a release:

```sh
igloo repo dotfiles set https://github.com/me/dotfiles \
  'curl -fsSL https://github.com/neovim/neovim/releases/latest/download/nvim-linux-x86_64.tar.gz | tar xz -C ~/.local --strip-components=1 && ./link.sh'
```

Neovim 0.10 and later copies to your laptop's clipboard through the terminal (OSC 52), which
works over `igloo shell`: set `vim.g.clipboard = 'osc52'` if it does not detect it. Run the
editor in `/workspace`; language servers and build tools run beside it in the same sandbox, with
the repository's warm dependencies.

## When Igloo is down

Igloo's copy of each repository is authoritative, and GitHub mirrors it: the default branch after
every merge and every `IGLOO_MIRROR_INTERVAL_SECONDS`, and the branch of every open change while
it is open. GitHub Actions keeps building every mirrored commit on its own. So when the server,
the worker or Igloo's own pipeline is broken (the usual way to need this is merging a change to
Igloo that breaks Igloo):

1. **Your work in a workspace is reachable only through Igloo.** Its changes live in the
   workspace's sealed layers on the worker. Push branches often: a pushed branch is mirrored to
   GitHub while its change is open. Work you did not push cannot be read while the server is
   down.
2. **SSH to the host Igloo runs on.** The server's data directory (`/var/lib/igloo` by
   `docs/deploy.md`) holds Igloo's copy of every repository as a bare repository,
   `repos/<repo id>.git`. It is current even when GitHub lags, and needs no running server:

   ```sh
   git clone /var/lib/igloo/repos/<repo id>.git fix && cd fix
   git remote add github git@github.com:roushou/igloo.git
   git push github origin/main:main        # bring GitHub up to the host's main if the mirror lagged
   git switch -c fix origin/main           # edit, commit the repair on top of main
   git push github fix:main                # a fast-forward; GitHub Actions builds it
   ```

   Without the host, clone GitHub instead; the repair must descend from the `main` Igloo has.
3. **Restart Igloo.** Within `IGLOO_MIRROR_INTERVAL_SECONDS` of starting, its sweep
   fetches GitHub's default branch and moves its own copy to it when it descends from Igloo's.
   A GitHub `main` that does not descend from Igloo's is never taken, and Igloo never overwrites
   it: the server logs "the forge's branch is not an ancestor of Igloo's; not mirrored". Make the
   repair a descendant of the host's `main` as above.
4. Reopen your workspaces with `igloo shell`; clone from Igloo again once it serves
   (`git clone <api url>/git/<repo id>.git`, with `igloo git-credential` as the credential
   helper, `docs/igloo-on-igloo.md`).

Plain SSH to the host is the break-glass for everything else: `journalctl -u igloo-control` for the
server's logs, and `docs/deploy.md` for restarting it.

## Attach to an agent's sandbox

```sh
igloo task take-over <task id>    # no new turn starts; a running turn finishes first
igloo shell <task id>             # a terminal in the task's sandbox, writable for you only
igloo task hand-back <task id>    # the agent continues, told what you changed
```

The task page shows the same: _Watch the sandbox_ is a read-only view (the server drops anything a
read-only terminal is sent), _Take over_ makes it writable. The sandbox keeps running while a task
is taken over. Handing back is possible once the turn that was running has ended; the agent's next
turn is told which commits you added since its own last one (subjects and a diff stat) and which
changes you left uncommitted. Your uncommitted work stays where it is; the task's next collection
commits it with the agent's. Only the person who took a task over can type in its sandbox or hand
it back; cancel the task to release a takeover nobody will finish.
