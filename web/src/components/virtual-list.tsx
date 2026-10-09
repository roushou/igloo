import { useVirtualizer } from "@tanstack/react-virtual";
import { type ReactNode, type Ref, useEffect, useImperativeHandle, useRef } from "react";
import { cn } from "@/lib/utils";

/** Lists longer than this are windowed; shorter ones render whole. */
export const WINDOW_FROM = 150;

export type ListHandle = { scrollToIndex: (index: number) => void };

type Props<T> = {
  items: readonly T[];
  /** The row height in pixels when rows are fixed; an estimate when `measure` is set. */
  rowHeight: number;
  /** Measure rows after rendering, for rows of varying height. */
  measure?: boolean;
  /** Keeps the end in view as items arrive, until the user scrolls away from it. */
  follow?: boolean;
  /** Called when the user scrolls away from the end while following. */
  onLeaveEnd?: () => void;
  renderItem: (item: T, index: number) => ReactNode;
  itemKey: (item: T, index: number) => string | number;
  className?: string;
  label: string;
  ref?: Ref<ListHandle>;
};

/**
 * A scrolling list that renders only the rows in view once it is long. Short lists render every
 * row, so their text stays selectable and searchable by the browser.
 */
export function VirtualList<T>({
  items,
  rowHeight,
  measure,
  follow,
  onLeaveEnd,
  renderItem,
  itemKey,
  className,
  label,
  ref,
}: Props<T>) {
  const scroller = useRef<HTMLDivElement>(null);
  const lastTop = useRef(0);
  const windowed = items.length > WINDOW_FROM;
  const virtualizer = useVirtualizer({
    count: windowed ? items.length : 0,
    getScrollElement: () => scroller.current,
    estimateSize: () => rowHeight,
    overscan: 20,
    getItemKey: (index) => itemKey(items[index] as T, index),
  });

  useImperativeHandle(
    ref,
    () => ({
      scrollToIndex(index) {
        if (windowed) {
          virtualizer.scrollToIndex(index, { align: "center" });
        } else {
          scroller.current
            ?.querySelector(`[data-index="${index}"]`)
            ?.scrollIntoView?.({ block: "center" });
        }
      },
    }),
    [windowed, virtualizer],
  );

  const count = items.length;
  useEffect(() => {
    if (!follow) return;
    const element = scroller.current;
    if (windowed) virtualizer.scrollToIndex(count - 1, { align: "end" });
    else if (element) element.scrollTop = element.scrollHeight;
  }, [follow, count, windowed, virtualizer]);

  return (
    <div
      ref={scroller}
      role="log"
      aria-label={label}
      className={cn("overflow-auto", className)}
      onWheel={(event) => {
        if (follow && event.deltaY < 0) onLeaveEnd?.();
      }}
      // Dragging the scrollbar, touch and the keyboard move the view too: scrolling up and away
      // from the end ends following, whatever did it.
      onScroll={(event) => {
        const element = event.currentTarget;
        const movedUp = element.scrollTop < lastTop.current - 2;
        lastTop.current = element.scrollTop;
        const fromEnd = element.scrollHeight - element.clientHeight - element.scrollTop;
        if (follow && movedUp && fromEnd > 48) onLeaveEnd?.();
      }}
    >
      {windowed ? (
        <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((row) => (
            <div
              key={row.key}
              data-index={row.index}
              ref={measure ? virtualizer.measureElement : undefined}
              className="absolute top-0 left-0 w-full"
              style={{
                transform: `translateY(${row.start}px)`,
                height: measure ? undefined : rowHeight,
              }}
            >
              {renderItem(items[row.index] as T, row.index)}
            </div>
          ))}
        </div>
      ) : (
        items.map((item, index) => (
          <div key={itemKey(item, index)} data-index={index}>
            {renderItem(item, index)}
          </div>
        ))
      )}
    </div>
  );
}
