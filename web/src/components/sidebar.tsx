import { Link, useNavigate } from "@tanstack/react-router";
import {
  GitPullRequest,
  Inbox,
  Keyboard,
  ListChecks,
  LogOut,
  type LucideIcon,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  Server,
  Settings,
} from "lucide-react";
import { motion } from "motion/react";
import { api } from "@/api/client";
import { ConnectionStatus } from "@/components/connection-status";
import { RepoSwitcher } from "@/components/repo-switcher";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip } from "@/components/ui/tooltip";
import { useNavCounts } from "@/lib/counts";
import { useMotion } from "@/lib/motion";
import { useShell } from "@/lib/shell";
import { useShortcuts } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";

const PAGES: {
  to: "/" | "/tasks" | "/changes" | "/system" | "/settings";
  label: string;
  icon: LucideIcon;
  keys: string;
}[] = [
  { to: "/", label: "Now", icon: Inbox, keys: "G N" },
  { to: "/tasks", label: "Tasks", icon: ListChecks, keys: "G T" },
  { to: "/changes", label: "Changes", icon: GitPullRequest, keys: "G C" },
  { to: "/system", label: "System", icon: Server, keys: "G S" },
  { to: "/settings", label: "Repository", icon: Settings, keys: "G R" },
];

/** The width of the sidebar with its labels, and with icons only. */
export const SIDEBAR_WIDTH = { expanded: 232, collapsed: 56 } as const;

/**
 * The navigation: repository switcher, search, the pages with their counts, and below them the
 * connection state and sign out. `compact` shows icons only; `sheet` is the narrow-viewport copy.
 */
export function SidebarContent({ compact, sheet }: { compact: boolean; sheet?: boolean }) {
  const navigate = useNavigate();
  const shell = useShell();
  const counts = useNavCounts();
  const shortcuts = useShortcuts();
  const close = () => shell.setSheetOpen(false);

  const count = (to: (typeof PAGES)[number]["to"]) =>
    to === "/" ? counts.waiting : to === "/changes" ? counts.openChanges : null;

  return (
    <div className="flex h-full flex-col gap-3 p-2">
      <div className={cn("flex items-center gap-1", compact && "flex-col")}>
        <div className="min-w-0 flex-1 self-stretch">
          <RepoSwitcher compact={compact} />
        </div>
        {sheet ? null : (
          <Tooltip content={compact ? "Expand the sidebar" : "Collapse the sidebar"} side="right">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={compact ? "Expand the sidebar" : "Collapse the sidebar"}
              onClick={shell.toggleCollapsed}
            >
              {compact ? <PanelLeftOpen /> : <PanelLeftClose />}
            </Button>
          </Tooltip>
        )}
      </div>

      <Tooltip content={compact ? "Search or run a command" : undefined} side="right">
        <button
          type="button"
          aria-label="Search or run a command"
          onClick={() => {
            close();
            shortcuts.openPalette();
          }}
          className={cn(
            "flex h-8 items-center gap-2 rounded-md border bg-card px-2 text-base text-muted-foreground hover:border-muted-foreground/40 hover:text-foreground",
            compact && "justify-center px-0",
          )}
        >
          <Search className="size-4 shrink-0" />
          {compact ? null : (
            <>
              <span className="flex-1 text-left">Search</span>
              <Kbd>⌘K</Kbd>
            </>
          )}
        </button>
      </Tooltip>

      <nav aria-label="Main" className="flex flex-col gap-0.5">
        {PAGES.map((page) => {
          const n = count(page.to);
          return (
            <Tooltip key={page.to} content={compact ? page.label : undefined} side="right">
              <Link
                to={page.to}
                onClick={close}
                activeOptions={{ exact: page.to === "/" }}
                aria-label={compact ? page.label : undefined}
                className={cn(
                  "group relative flex h-8 items-center gap-2.5 rounded-md px-2 text-base text-muted-foreground hover:bg-accent hover:text-foreground data-[status=active]:bg-accent data-[status=active]:font-medium data-[status=active]:text-foreground",
                  compact && "justify-center px-0",
                )}
              >
                <page.icon className="size-4 shrink-0" />
                {compact ? null : <span className="flex-1">{page.label}</span>}
                {n ? (
                  <span
                    className={cn(
                      "tabular text-sm",
                      page.to === "/"
                        ? "rounded-full bg-expedition px-1.5 font-medium text-expedition-foreground"
                        : "text-muted-foreground",
                      compact && "absolute top-0.5 right-1 px-1 text-xs leading-4",
                    )}
                  >
                    {n}
                    <span className="sr-only">{page.to === "/" ? " waiting on you" : " open"}</span>
                  </span>
                ) : null}
              </Link>
            </Tooltip>
          );
        })}
      </nav>

      <div className="mt-auto flex flex-col gap-0.5">
        <Tooltip content={compact ? "Keyboard shortcuts" : undefined} side="right">
          <button
            type="button"
            aria-label="Keyboard shortcuts"
            onClick={() => {
              close();
              shortcuts.openHelp();
            }}
            className={cn(
              "flex h-8 items-center gap-2.5 rounded-md px-2 text-base text-muted-foreground hover:bg-accent hover:text-foreground",
              compact && "justify-center px-0",
            )}
          >
            <Keyboard className="size-4 shrink-0" />
            {compact ? null : (
              <>
                <span className="flex-1 text-left">Shortcuts</span>
                <Kbd>?</Kbd>
              </>
            )}
          </button>
        </Tooltip>
        <Tooltip content={compact ? "Sign out" : undefined} side="right">
          <button
            type="button"
            aria-label="Sign out"
            onClick={() => {
              api.tokens.clear();
              void navigate({ to: "/signin" });
            }}
            className={cn(
              "flex h-8 items-center gap-2.5 rounded-md px-2 text-base text-muted-foreground hover:bg-accent hover:text-foreground",
              compact && "justify-center px-0",
            )}
          >
            <LogOut className="size-4 shrink-0" />
            {compact ? null : <span className="flex-1 text-left">Sign out</span>}
          </button>
        </Tooltip>
        <div className={cn("h-8 items-center", compact ? "flex justify-center" : "flex")}>
          <ConnectionStatus compact={compact} />
        </div>
      </div>
    </div>
  );
}

/** The sidebar of a wide viewport: it keeps its place and animates between its two widths. */
export function Sidebar() {
  const { collapsed } = useShell();
  const { transition } = useMotion();
  return (
    <motion.aside
      aria-label="Sidebar"
      initial={false}
      animate={{ width: collapsed ? SIDEBAR_WIDTH.collapsed : SIDEBAR_WIDTH.expanded }}
      transition={transition(0.22)}
      className="sticky top-0 hidden h-dvh shrink-0 overflow-hidden lg:block"
    >
      <SidebarContent compact={collapsed} />
    </motion.aside>
  );
}
