import { Markdown } from "./Markdown";
import { cn } from "@/lib/utils";
import type { ChatTurn } from "@/types";

export function MessageBubble({ turn }: { turn: ChatTurn }) {
  const isUser = turn.role === "user";
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
        {isUser ? (
          <p className="whitespace-pre-wrap text-sm leading-relaxed">{turn.text}</p>
        ) : (
          <Markdown content={turn.text} />
        )}
      </div>
    </div>
  );
}
