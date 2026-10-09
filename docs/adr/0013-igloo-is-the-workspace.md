# 0013. Igloo is the workspace; the forge becomes a mirror

- Status: accepted
- Date: 2026-10-09
- Amends: ADR 0011 (the forge stops being the git host)

## Context

Working on a repository through Igloo means keeping several copies in step by hand: a clone on the
owner's laptop, the forge, the host's deployment checkout, Igloo's mirror, and each sandbox's
checkout. Branches pile up on the forge, it is unclear which commit is live, and an agent's work
lives in a sandbox the owner cannot enter. Igloo already builds what a development environment
needs: sandboxes forked from a warm snapshot of the repository in under a second, with
dependencies built, secrets granted and the network allowed.

## Decision

- **Igloo is where the code is worked on.** A workspace is a long-lived sandbox, owned by a
  person, forked from the warm snapshot of a branch. The owner reaches it through a terminal in
  the console or `igloo shell` from any machine, and runs their editor (Neovim) and coding tools
  inside it. Agents work in the same kind of sandbox, and a person can attach to an agent's
  sandbox to watch it or take over.
- **A workspace outlives its sandbox.** Stopping it seals its changes into a layer; opening it
  again resumes from that layer. Nothing on the owner's machine needs to be kept in sync.
- **Igloo hosts the repository's git.** Igloo's copy of each repository is the authoritative
  one: workspaces, agents and people clone from and push to Igloo. A push to a branch can open a
  change; the default branch moves only through a merge.
- **The forge becomes a mirror.** Igloo pushes the default branch (and, while useful, change
  branches) to the forge, for backup, visibility and the independent break-glass CI. The forge is
  never the source of truth again.
- **No editor of our own.** The terminal carries the owner's editor; in-browser editing, when it
  comes, embeds an existing editor.

## Consequences

The laptop clone becomes optional, and "what is live" has one answer: the default branch in
Igloo, at the last deploy. Igloo needs interactive terminals (a worker protocol change), workspace
persistence, and a git endpoint with authentication. Igloo developing itself on the host it runs
on becomes a single point of failure: the forge mirror and plain SSH to the host stay as the
escape hatch.

## Alternatives considered

- Keep the laptop clone and the forge as the git host: the manual syncing this decision exists
  to remove.
- A hosted cloud workspace product: a second system holding the code, and no shared environment
  with Igloo's agents.
- Build an editor into Igloo: years of work for less than Neovim in a terminal already gives.
