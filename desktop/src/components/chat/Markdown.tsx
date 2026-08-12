import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn, numberHeadings } from "@/lib/utils";

type MarkdownProps = {
  content: string;
  className?: string;
  /** Render-layer heading auto-numbering (1 / 1.1 / 1.1.2…) (#4062). */
  numberHeadings?: boolean;
};

/**
 * Markdown renderer wrapping react-markdown. Styled with Tailwind via a single
 * `prose-chat` container — keeps chat bubbles compact while supporting GFM
 * tables, lists, code blocks, etc. Wikilink handling lands in a later stage.
 */
export function Markdown({ content, className, numberHeadings: numbered }: MarkdownProps) {
  const rendered = numbered ? numberHeadings(content) : content;
  return (
    <div className={cn("vp-md text-sm leading-relaxed", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          // Code blocks — monospace with subtle background.
          pre: ({ children }) => (
            <pre className="my-2 overflow-x-auto rounded-md border border-border bg-muted p-3 text-xs vp-scroll">
              {children}
            </pre>
          ),
          code: ({ children, className: cls }) => (
            <code className={cn("rounded bg-muted px-1 py-0.5 text-xs", !cls && "font-mono", cls)}>
              {children}
            </code>
          ),
          a: ({ children, href }) => (
            <a href={href} target="_blank" rel="noreferrer" className="text-primary underline underline-offset-2 hover:opacity-80">
              {children}
            </a>
          ),
          table: ({ children }) => (
            <div className="my-2 overflow-x-auto vp-scroll">
              <table className="w-full border-collapse text-xs">{children}</table>
            </div>
          ),
          th: ({ children }) => (
            <th className="border border-border bg-muted px-2 py-1 text-left font-medium">{children}</th>
          ),
          td: ({ children }) => <td className="border border-border px-2 py-1">{children}</td>,
        }}
      >
        {rendered}
      </ReactMarkdown>
    </div>
  );
}
