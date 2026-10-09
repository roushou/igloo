import { Link, Outlet, useNavigate } from "@tanstack/react-router";
import { api } from "@/api/client";
import { CommandPalette } from "@/components/command-palette";
import { ConnectionStatus } from "@/components/connection-status";
import { RepoSwitcher } from "@/components/repo-switcher";
import { Button } from "@/components/ui/button";
import { EventsProvider } from "@/lib/events";
import { SelectedRepoProvider } from "@/lib/selected-repo";

const PAGES = [
  { to: "/", label: "Now" },
  { to: "/tasks", label: "Tasks" },
  { to: "/changes", label: "Changes" },
  { to: "/system", label: "System" },
  { to: "/settings", label: "Repository" },
] as const;

/** The signed-in layout: a left rail with the repository switcher and the pages. */
export function AppShell() {
  const navigate = useNavigate();
  return (
    <EventsProvider>
      <SelectedRepoProvider>
        <div className="flex min-h-screen">
          <nav
            aria-label="Main"
            className="sticky top-0 flex h-screen w-56 shrink-0 flex-col gap-4 border-r bg-card p-3"
          >
            <p className="px-2 font-display text-xl font-semibold tracking-tight">Igloo</p>
            <RepoSwitcher />
            <ul className="flex flex-col gap-1">
              {PAGES.map((page) => (
                <li key={page.to}>
                  <Link
                    to={page.to}
                    activeOptions={{ exact: page.to === "/" }}
                    className="block rounded-md px-2 py-1.5 text-sm hover:bg-accent data-[status=active]:bg-muted data-[status=active]:font-medium"
                  >
                    {page.label}
                  </Link>
                </li>
              ))}
            </ul>
            <p className="px-2 text-xs text-muted-foreground">
              <kbd className="rounded border px-1">⌘K</kbd> opens a resource by id
            </p>
            <ConnectionStatus />
            <Button
              variant="ghost"
              size="sm"
              className="justify-start"
              onClick={() => {
                api.tokens.clear();
                void navigate({ to: "/signin" });
              }}
            >
              Sign out
            </Button>
          </nav>
          <main className="min-w-0 flex-1 p-8">
            <Outlet />
          </main>
        </div>
        <CommandPalette />
      </SelectedRepoProvider>
    </EventsProvider>
  );
}
