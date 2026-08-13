import { Markdown } from "./Markdown";
import { cn } from "@/lib/utils";
import type { ChatTurn } from "@/types";

export function MessageBubble({ turn }: { turn: ChatTurn }) {
  const isUser = turn.role === "user";
  const attachments = turn.attachments?.filter((a) => a.dataUrl) ?? [];
  return (
    <div className={cn("flex w-full gap-3 px-4 py-3", isUser ? "justify-end" : "justify-start")}>
      <div
        className={cn(
          "max-w-[80%] rounded-2xl px-4 py-2.5",
          isUser
            ? "bg-primary text-primary-foreground rounded-br-md"
            : "bg-card border border-border rounded-bl-md"
        )}
      >
        {/* Image attachments picked in the composer (#4074). */}
        {attachments.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-2">
            {attachments.map((a, i) => (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                key={i}
                src={a.dataUrl}
                alt={a.name ?? "attachment"}
                className="max-h-48 max-w-full rounded-lg object-contain"
              />
            ))}
          </div>
        )}
        {isUser ? (
          <p className="whitespace-pre-wrap text-sm leading-relaxed">{turn.text}</p>
        ) : (
          <Markdown content={turn.text} />
        )}
      </div>
    </div>
  );
}
