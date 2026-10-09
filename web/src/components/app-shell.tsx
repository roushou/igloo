import { Outlet } from "@tanstack/react-router";
import { CommandPalette } from "@/components/command-palette";
import { ShortcutsDialog } from "@/components/shortcuts-dialog";
import { Sidebar, SidebarContent } from "@/components/sidebar";
import { Modal } from "@/components/ui/dialog";
import { TooltipProvider } from "@/components/ui/tooltip";
import { CommandsProvider } from "@/lib/commands";
import { EventsProvider } from "@/lib/events";
import { useIsWide } from "@/lib/media";
import { SelectedRepoProvider } from "@/lib/selected-repo";
import { ShellProvider, useShell } from "@/lib/shell";
import { ShortcutsProvider } from "@/lib/shortcuts";

/**
 * The signed-in layout: the sidebar, and the page in a panel beside it. On a narrow viewport the
 * sidebar is a sheet that the page header opens.
 */
export function AppShell() {
  return (
    <EventsProvider>
      <SelectedRepoProvider>
        <ShellProvider>
          <CommandsProvider>
            <ShortcutsProvider>
              <TooltipProvider>
                <Frame />
                <CommandPalette />
                <ShortcutsDialog />
              </TooltipProvider>
            </ShortcutsProvider>
          </CommandsProvider>
        </ShellProvider>
      </SelectedRepoProvider>
    </EventsProvider>
  );
}

function Frame() {
  const shell = useShell();
  const wide = useIsWide();
  return (
    <div className="flex h-dvh overflow-hidden bg-background">
      <Sidebar />
      <main className="flex min-w-0 flex-1 flex-col bg-card lg:my-2 lg:mr-2 lg:overflow-clip lg:rounded-xl lg:border">
        <Outlet />
      </main>
      <Modal
        open={shell.sheetOpen && !wide}
        onOpenChange={shell.setSheetOpen}
        label="Menu"
        placement="sheet"
      >
        <SidebarContent compact={false} sheet />
      </Modal>
    </div>
  );
}
