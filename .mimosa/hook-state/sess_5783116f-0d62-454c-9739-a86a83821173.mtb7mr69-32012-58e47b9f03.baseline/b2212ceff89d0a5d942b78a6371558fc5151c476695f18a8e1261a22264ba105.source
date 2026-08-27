import type { HTMLAttributes } from "react";
import { cn } from "@/lib/utils";

/**
 * Minimal scroll-area — a styled overflow container. Real shadcn uses radix
 * scroll-area for cross-browser scrollbar styling; this is a lightweight
 * substitute that hides scrollbars on WebKit and stylizes them via CSS.
 */
export type ScrollAreaProps = HTMLAttributes<HTMLDivElement>;

export function ScrollArea({ className, children, ...props }: ScrollAreaProps) {
  return (
    <div
      className={cn("overflow-auto vp-scroll", className)}
      {...props}
    >
      {children}
    </div>
  );
}
