import { Modal } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { SHORTCUTS, useShortcuts } from "@/lib/shortcuts";

/** `?`: every keyboard shortcut of the console. */
export function ShortcutsDialog() {
  const { helpOpen, setHelpOpen } = useShortcuts();
  return (
    <Modal open={helpOpen} onOpenChange={setHelpOpen} label="Keyboard shortcuts">
      <div className="flex max-h-[70vh] flex-col gap-5 overflow-y-auto p-5">
        <h2 className="font-display text-lg font-semibold tracking-tight">Keyboard shortcuts</h2>
        {SHORTCUTS.map((group) => (
          <section key={group.group} aria-label={group.group} className="flex flex-col gap-1">
            <h3 className="caption pb-1">{group.group}</h3>
            <ul className="flex flex-col">
              {group.items.map((item) => (
                <li key={item.label} className="flex items-center justify-between gap-4 py-1.5">
                  <span className="text-base">{item.label}</span>
                  <span className="flex items-center gap-1 text-sm text-muted-foreground">
                    {item.keys.map((key, index) => (
                      <span key={key} className="flex items-center gap-1">
                        {index > 0 ? <span>{item.sequence ? "then" : "+"}</span> : null}
                        <Kbd>{key}</Kbd>
                      </span>
                    ))}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </Modal>
  );
}
