import { CircleAlert, CircleCheck, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";

/** What a toast says. `tone` defaults to `success`. */
export type ToastInput = { title: string; description?: string; tone?: "success" | "error" };

type Toast = ToastInput & { id: number };

type Toasts = { show: (toast: ToastInput) => void };

const Context = createContext<Toasts | null>(null);

/** How long a toast stays, in milliseconds; errors stay twice as long. */
const LIFETIME_MS = 4_000;
const MAX_SHOWN = 4;

/** Holds the toasts of the app and draws them in the bottom corner. */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const next = useRef(0);
  const { transition } = useMotion();

  const dismiss = useCallback((id: number) => {
    setToasts((all) => all.filter((toast) => toast.id !== id));
  }, []);
  const show = useCallback((toast: ToastInput) => {
    next.current += 1;
    const id = next.current;
    setToasts((all) => [...all.slice(-(MAX_SHOWN - 1)), { ...toast, id }]);
  }, []);
  const value = useMemo(() => ({ show }), [show]);

  return (
    <Context.Provider value={value}>
      {children}
      <section
        aria-label="Notifications"
        className="pointer-events-none fixed inset-x-3 bottom-3 z-[70] flex flex-col items-end gap-2 sm:inset-x-auto sm:right-4 sm:bottom-4"
      >
        <AnimatePresence initial={false}>
          {toasts.map((toast) => (
            <motion.div
              key={toast.id}
              layout="position"
              initial={{ opacity: 0, y: 12, scale: 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, scale: 0.98 }}
              transition={transition(0.2)}
              className="pointer-events-auto w-full sm:w-80"
            >
              <ToastCard toast={toast} onDismiss={dismiss} />
            </motion.div>
          ))}
        </AnimatePresence>
      </section>
    </Context.Provider>
  );
}

function ToastCard({ toast, onDismiss }: { toast: Toast; onDismiss: (id: number) => void }) {
  const failed = toast.tone === "error";
  useEffect(() => {
    const timer = setTimeout(() => onDismiss(toast.id), failed ? LIFETIME_MS * 2 : LIFETIME_MS);
    return () => clearTimeout(timer);
  }, [onDismiss, toast.id, failed]);
  const Icon = failed ? CircleAlert : CircleCheck;
  return (
    <div
      role="status"
      className="flex items-start gap-3 rounded-lg border bg-card p-3 shadow-float"
    >
      <Icon className={cn("mt-0.5 size-4 shrink-0", failed ? "text-failed" : "text-passed")} />
      <div className="min-w-0 flex-1">
        <p className="text-base font-medium">{toast.title}</p>
        {toast.description ? (
          <p className="mt-0.5 text-sm text-muted-foreground">{toast.description}</p>
        ) : null}
      </div>
      <button
        type="button"
        aria-label="Dismiss"
        className="-m-1 rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
        onClick={() => onDismiss(toast.id)}
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}

/** Shows a toast: `useToast().show({ title: "Merged" })`. */
export function useToast(): Toasts {
  const value = useContext(Context);
  if (!value) throw new Error("useToast needs a ToastProvider");
  return value;
}
