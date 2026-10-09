import { AnimatePresence, motion } from "motion/react";
import { Dialog as Radix } from "radix-ui";
import type { ReactNode } from "react";
import { useMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";

type Placement = "center" | "palette" | "sheet";

const PLACEMENT: Record<Placement, string> = {
  center: "inset-x-4 top-[18vh] mx-auto max-w-md rounded-xl border",
  palette: "inset-x-4 top-[12vh] mx-auto max-w-xl overflow-hidden rounded-xl border",
  sheet: "inset-y-0 left-0 w-72 max-w-[85vw] border-r",
};

/**
 * A modal on a dimmed page. It traps focus, closes on Escape, and enters and leaves with a short
 * transition. `label` names it for assistive technology; the content shows its own heading.
 */
export function Modal({
  open,
  onOpenChange,
  label,
  placement = "center",
  role,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  label: string;
  placement?: Placement;
  role?: "alertdialog";
  children: ReactNode;
}) {
  const { transition } = useMotion();
  const from = placement === "sheet" ? { x: "-100%" } : { y: -8, scale: 0.98 };
  return (
    <Radix.Root open={open} onOpenChange={onOpenChange}>
      <AnimatePresence>
        {open ? (
          <Radix.Portal forceMount>
            <Radix.Overlay asChild forceMount>
              <motion.div
                className="fixed inset-0 z-50 bg-overlay"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={transition(0.14)}
              />
            </Radix.Overlay>
            <Radix.Content
              asChild
              forceMount
              aria-describedby={undefined}
              {...(role ? { role } : {})}
            >
              <motion.div
                className={cn("fixed z-50 bg-card shadow-float outline-none", PLACEMENT[placement])}
                initial={{ opacity: 0, ...from }}
                animate={{ opacity: 1, x: 0, y: 0, scale: 1 }}
                exit={{ opacity: 0, ...from }}
                transition={transition(0.18)}
              >
                <Radix.Title className="sr-only">{label}</Radix.Title>
                {children}
              </motion.div>
            </Radix.Content>
          </Radix.Portal>
        ) : null}
      </AnimatePresence>
    </Radix.Root>
  );
}

/** The heading, text and buttons of a small confirmation, for use inside a `Modal`. */
export function ModalBody({
  title,
  description,
  children,
}: {
  title: string;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-4 p-5">
      <div className="flex flex-col gap-1.5">
        <h2 className="font-display text-lg font-semibold tracking-tight">{title}</h2>
        {description ? <div className="text-base text-muted-foreground">{description}</div> : null}
      </div>
      <div className="flex justify-end gap-2">{children}</div>
    </div>
  );
}
