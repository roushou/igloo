import { useReducedMotion } from "motion/react";

/** How long the shortest transitions last, in seconds. */
const QUICK = 0.16;

/**
 * Transitions for `motion` components. Under `prefers-reduced-motion` every duration is zero, so
 * state changes still happen but nothing travels or fades.
 */
export function useMotion() {
  const reduced = useReducedMotion() ?? false;
  return {
    reduced,
    /** A short ease-out transition; instant when the user asks for less motion. */
    transition: (seconds = QUICK) =>
      reduced ? { duration: 0 } : { duration: seconds, ease: [0.25, 1, 0.5, 1] as const },
  };
}
