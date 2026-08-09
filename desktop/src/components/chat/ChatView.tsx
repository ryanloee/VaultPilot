import { useEffect, useRef, useState } from "react";
import { useChatStore } from "@/lib/store";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ScrollArea } from "@/components/ui/scroll-area";
import { MessageBubble } from "./MessageBubble";

export function ChatView() {
  const { chatState, currentSessionId, turns, sending, status, error, load, send, newSession } =
    useChatStore();
  const [input, setInput] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    load();
  }, [load]);

  const currentSession = chatState?.sessions.find((s) => s.id === currentSessionId);

  // Auto-scroll to bottom on new messages.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [turns, status]);

  const handleSend = () => {
    if (!input.trim() || sending) return;
    send(input);
    setInput("");
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Ctrl/Cmd+Enter to send; plain Enter for newline.
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="flex h-full flex-col">
      {/* Messages */}
      <ScrollArea className="flex-1" >
        <div ref={scrollRef} className="mx-auto max-w-3xl py-4">
          {turns.length === 0 && !sending && (
            <div className="flex h-full min-h-[40vh] flex-col items-center justify-center text-center">
              <h2 className="text-2xl font-semibold tracking-tight">
                {currentSession?.title || "开始对话"}
              </h2>
              <p className="mt-2 text-sm text-muted-foreground">
                输入问题，Ctrl+Enter 发送
              </p>
            </div>
          )}
          {turns.map((m, i) => (
            <MessageBubble key={m.id ?? i} turn={m} />
          ))}
          {sending && (
            <div className="flex justify-start px-4 py-3">
              <div className="max-w-[80%] rounded-2xl rounded-bl-md border border-border bg-card px-4 py-2.5">
                {status ? (
                  <span className="text-xs text-muted-foreground">{status.detail}</span>
                ) : (
                  <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                    <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-muted-foreground" />
                    正在思考…
                  </span>
                )}
              </div>
            </div>
          )}
          {error && (
            <div className="mx-auto my-2 max-w-3xl px-4">
              <p className="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
                {error}
              </p>
            </div>
          )}
        </div>
      </ScrollArea>

      {/* Composer */}
      <div className="border-t border-border bg-background p-3">
        <div className="mx-auto flex max-w-3xl items-end gap-2">
          <Textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="输入问题…（Ctrl+Enter 发送）"
            rows={2}
            className="min-h-[44px] flex-1 resize-none"
            disabled={sending}
          />
          <Button onClick={handleSend} disabled={sending || !input.trim()} size="default">
            发送
          </Button>
          <Button onClick={newSession} variant="ghost" size="icon" title="新会话">
            +
          </Button>
        </div>
      </div>
    </div>
  );
}
