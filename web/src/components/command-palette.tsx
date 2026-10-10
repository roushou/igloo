import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import {
  CornerDownLeft,
  FolderGit2,
  GitPullRequest,
  Inbox,
  Keyboard,
  ListChecks,
  LogOut,
  PanelLeft,
  Plus,
  Server,
  Settings,
  SquareTerminal,
} from "lucide-react";
import { useMemo, useState } from "react";
import { api } from "@/api/client";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Modal } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { type Command as PaletteCommand, usePageCommands } from "@/lib/commands";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { ResourceId } from "@/lib/resource-id";
import { useSelectedRepo } from "@/lib/selected-repo";
import { useShell } from "@/lib/shell";
import { useShortcuts } from "@/lib/shortcuts";

const LABELS = {
  repo: "repository",
  chg: "change",
  task: "task",
  run: "run",
  job: "job",
  sbx: "sandbox",
  wsp: "workspace",
} as const;

/**
 * ⌘K / Ctrl+K: search the commands, open the resource whose id is pasted in, go to a page,
 * switch repository, and run what the page on screen offers, such as merging the open change.
 */
export function CommandPalette() {
  const { paletteOpen, setPaletteOpen, openHelp } = useShortcuts();
  const [text, setText] = useState("");
  const navigate = useNavigate();
  const repos = useQuery(queries.repos());
  const selected = useSelectedRepo();
  const shell = useShell();
  const pageCommands = usePageCommands();

  const close = () => {
    setPaletteOpen(false);
    setText("");
  };

  const resource = ResourceId.parse(text);

  const openResource = (resource: ResourceId) => {
    close();
    switch (resource.kind) {
      case "repo":
        selected.select(resource.id);
        return navigate({ to: "/" });
      case "chg":
        return navigate({ to: "/changes/$id", params: { id: resource.id } });
      case "task":
        return navigate({ to: "/tasks/$id", params: { id: resource.id } });
      case "run":
        return navigate({ to: "/runs/$id", params: { id: resource.id } });
      case "job":
        return navigate({ to: "/jobs/$id", params: { id: resource.id } });
      case "sbx":
        return navigate({ to: "/system", search: { sandbox: resource.id } });
      case "wsp":
        return navigate({ to: "/workspaces/$id", params: { id: resource.id } });
    }
  };

  const global = useMemo<PaletteCommand[]>(
    () => [
      {
        id: "create-task",
        label: "Create task",
        group: "Create",
        icon: Plus,
        shortcut: "C",
        keywords: ["new", "agent", "goal"],
        run: () => void navigate({ to: "/tasks", search: { new: true } }),
      },
      {
        id: "go-now",
        label: "Go to Now",
        group: "Go to",
        icon: Inbox,
        shortcut: "G N",
        run: () => void navigate({ to: "/" }),
      },
      {
        id: "go-tasks",
        label: "Go to Tasks",
        group: "Go to",
        icon: ListChecks,
        shortcut: "G T",
        run: () => void navigate({ to: "/tasks" }),
      },
      {
        id: "go-changes",
        label: "Go to Changes",
        group: "Go to",
        icon: GitPullRequest,
        shortcut: "G C",
        run: () => void navigate({ to: "/changes" }),
      },
      {
        id: "go-workspaces",
        label: "Go to Workspaces",
        group: "Go to",
        icon: SquareTerminal,
        shortcut: "G W",
        keywords: ["terminal", "shell"],
        run: () => void navigate({ to: "/workspaces" }),
      },
      {
        id: "go-system",
        label: "Go to System",
        group: "Go to",
        icon: Server,
        shortcut: "G S",
        run: () => void navigate({ to: "/system", search: { sandbox: undefined } }),
      },
      {
        id: "go-repo",
        label: "Go to Repository",
        group: "Go to",
        icon: Settings,
        shortcut: "G R",
        keywords: ["secrets", "settings"],
        run: () => void navigate({ to: "/settings" }),
      },
      ...(repos.data ?? []).map((repo) => ({
        id: `repo-${repo.id}`,
        label: `Switch to ${format.repoName(repo.location)}`,
        group: "Repositories",
        icon: FolderGit2,
        keywords: [repo.id, repo.location],
        run: () => selected.select(repo.id),
      })),
      {
        id: "sidebar",
        label: "Collapse or expand the sidebar",
        group: "Interface",
        icon: PanelLeft,
        shortcut: "[",
        run: shell.toggleCollapsed,
      },
      {
        id: "help",
        label: "Show keyboard shortcuts",
        group: "Interface",
        icon: Keyboard,
        shortcut: "?",
        run: openHelp,
      },
      {
        id: "sign-out",
        label: "Sign out",
        group: "Interface",
        icon: LogOut,
        run: () => {
          api.tokens.clear();
          void navigate({ to: "/signin" });
        },
      },
    ],
    [navigate, repos.data, selected, shell.toggleCollapsed, openHelp],
  );

  const groups = useMemo(() => {
    const byGroup = new Map<string, PaletteCommand[]>();
    for (const command of [...pageCommands, ...global]) {
      byGroup.set(command.group, [...(byGroup.get(command.group) ?? []), command]);
    }
    return [...byGroup.entries()];
  }, [pageCommands, global]);

  return (
    <Modal
      open={paletteOpen}
      onOpenChange={(open) => (open ? setPaletteOpen(true) : close())}
      label="Command palette"
      placement="palette"
    >
      <Command label="Command palette" className="max-h-[70vh]">
        <div className="border-b px-4">
          <CommandInput
            value={text}
            onValueChange={setText}
            placeholder="Search, paste an id, or run a command"
          />
        </div>
        <CommandList>
          {resource ? (
            <CommandGroup heading="Open" forceMount>
              <CommandItem
                value={resource.id}
                forceMount
                disabled={
                  resource.kind === "repo" && !repos.data?.some((r) => r.id === resource.id)
                }
                onSelect={() => openResource(resource)}
              >
                <CornerDownLeft />
                <span>Open {LABELS[resource.kind]}</span>
                <code className="font-mono text-sm text-muted-foreground">{resource.id}</code>
              </CommandItem>
            </CommandGroup>
          ) : null}
          {groups.map(([group, commands]) => (
            <CommandGroup key={group} heading={group}>
              {commands.map((command) => (
                <CommandItem
                  key={command.id}
                  value={`${command.group} ${command.label}`}
                  keywords={command.keywords}
                  disabled={Boolean(command.unmet)}
                  onSelect={() => {
                    close();
                    command.run();
                  }}
                >
                  {command.icon ? <command.icon /> : null}
                  <span className="min-w-0 flex-1">
                    {command.label}
                    {command.unmet ? (
                      <span className="block truncate text-sm text-muted-foreground">
                        {command.unmet}
                      </span>
                    ) : null}
                  </span>
                  {command.unmet ? null : command.shortcut ? (
                    <span className="flex gap-1">
                      {command.shortcut.split(" ").map((key) => (
                        <Kbd key={key}>{key}</Kbd>
                      ))}
                    </span>
                  ) : null}
                </CommandItem>
              ))}
            </CommandGroup>
          ))}
          {resource ? null : (
            <CommandEmpty>Nothing matches. Paste an id to open a resource.</CommandEmpty>
          )}
        </CommandList>
        <div className="hidden items-center gap-4 border-t px-4 py-2 text-sm text-muted-foreground sm:flex">
          <span className="flex items-center gap-1.5">
            <Kbd>↑</Kbd>
            <Kbd>↓</Kbd> move
          </span>
          <span className="flex items-center gap-1.5">
            <Kbd>↵</Kbd> run
          </span>
          <span className="flex items-center gap-1.5">
            <Kbd>esc</Kbd> close
          </span>
        </div>
      </Command>
    </Modal>
  );
}
