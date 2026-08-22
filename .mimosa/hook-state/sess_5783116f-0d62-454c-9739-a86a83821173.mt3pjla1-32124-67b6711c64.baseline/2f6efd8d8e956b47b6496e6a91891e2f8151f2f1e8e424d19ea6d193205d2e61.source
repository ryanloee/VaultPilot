import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import { Markdown } from "./Markdown";
import { FileIcon } from "@/components/layout/icons";
import { cn } from "@/lib/utils";
import type { ChatAttachment, ChatTurn } from "@/types";

/**
 * Renders one attachment image. Prefers the in-memory base64 `dataUrl`
 * (optimistic rendering for a just-sent turn); otherwise loads the bytes back
 * from the persisted vault path via `read_image_preview` (#4083). A dead path
 * (e.g. pre-fix temp-dir attachment) renders nothing instead of a broken img.
 */
function AttachmentImage({ attachment }: { attachment: ChatAttachment }) {
  const [preview, setPreview] = useState<string | undefined>(attachment.dataUrl);

  useEffect(() => {
    if (attachment.dataUrl) {
      setPreview(attachment.dataUrl);
      return;
    }
    if (!attachment.path) {
      setPreview(undefined);
      return;
    }
    let cancelled = false;
    api
      .readImagePreview(attachment.path)
      .then((p) => {
        if (!cancelled) setPreview(p);
      })
      .catch(() => {
        // Path may be gone (temp-dir wipe); render nothing.
      });
    return () => {
      cancelled = true;
    };
  }, [attachment.path, attachment.dataUrl]);

  if (!preview) return null;
  return (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={preview}
      alt={attachment.name ?? "attachment"}
      className="max-h-48 max-w-full rounded-lg object-contain"
    />
  );
}

export function MessageBubble({ turn }: { turn: ChatTurn }) {
  const isUser = turn.role === "user";
  const attachments = turn.attachments?.filter((a) => a.dataUrl || a.path) ?? [];
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
        {/* Image/file attachments picked in the composer (#4074). */}
        {attachments.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-2">
            {attachments.map((a, i) =>
              a.type?.startsWith("image/") ? (
                <AttachmentImage key={i} attachment={a} />
              ) : (
                <div
                  key={i}
                  className="flex max-w-56 items-center gap-2 rounded-lg border border-border bg-background/60 px-3 py-2 text-xs"
                >
                  <FileIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="truncate">{a.name ?? a.path ?? "附件"}</span>
                </div>
              )
            )}
          </div>
        )}
        {/* Both user and assistant turns render markdown — plain text falls
            through react-markdown unchanged, so nothing breaks for
            non-markdown input. */}
        <Markdown content={turn.text} />
      </div>
    </div>
  );
}
