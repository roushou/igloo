import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import {
  CommandDialog,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { queries } from "@/lib/queries";
import { ResourceId } from "@/lib/resource-id";
import { useSelectedRepo } from "@/lib/selected-repo";

const LABELS = {
  repo: "repository",
  chg: "change",
  task: "task",
  run: "run",
  job: "job",
  sbx: "sandbox",
} as const;

/** ⌘K / Ctrl+K: opens the resource whose id is pasted in. */
export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const navigate = useNavigate();
  const repos = useQuery(queries.repos());
  const selected = useSelectedRepo();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setOpen((value) => !value);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const resource = ResourceId.parse(text);

  const openResource = (resource: ResourceId) => {
    setOpen(false);
    setText("");
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
    }
  };

  return (
    <CommandDialog open={open} onOpenChange={setOpen} title="Command palette">
      <CommandInput
        value={text}
        onValueChange={setText}
        placeholder="Paste a repo_, chg_, task_, run_, job_ or sbx_ id"
      />
      <CommandList>
        {resource ? (
          <CommandItem
            value={resource.id}
            disabled={resource.kind === "repo" && !repos.data?.some((r) => r.id === resource.id)}
            onSelect={() => openResource(resource)}
          >
            Open {LABELS[resource.kind]} <code className="text-xs">{resource.id}</code>
          </CommandItem>
        ) : (
          <CommandEmpty>Paste a resource id to open it.</CommandEmpty>
        )}
      </CommandList>
    </CommandDialog>
  );
}
