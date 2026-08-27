import { useEffect, useRef, useState } from "react";
import { useChatStore } from "@/lib/store";
import { api } from "@/lib/tauri";
import type { ChatAttachment } from "@/types";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ScrollArea } from "@/components/ui/scroll-area";
import { MessageBubble } from "./MessageBubble";
import { Waveform } from "./Waveform";
import { FileIcon, ImageIcon, MicIcon, PlusIcon, StopIcon } from "@/components/layout/icons";
import { cn } from "@/lib/utils";

/** Read a File/Blob as a base64 data URL (works in WebView + browser). */
export function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error ?? new Error("read failed"));
    reader.readAsDataURL(blob);
  });
}

/** Strip the `data:<mime>;base64,` prefix → raw base64 payload for the bridge. */
export function dataUrlToBase64(dataUrl: string): string {
  const comma = dataUrl.indexOf(",");
  return comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
}

/** Live "thinking" indicator: shows current stage + a running elapsed timer so
 *  long AI calls never look frozen. */
function ThinkingIndicator({ detail }: { detail: string | null }) {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    const started = Date.now();
    const timer = setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => clearInterval(timer);
  }, []);

  const mm = String(Math.floor(elapsed / 60)).padStart(2, "0");
  const ss = String(elapsed % 60).padStart(2, "0");

  return (
    <div className="flex justify-start px-4 py-3">
      <div className="max-w-[80%] rounded-2xl rounded-bl-md border border-border bg-card px-4 py-2.5">
        <div className="flex items-center gap-2">
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
            {detail ?? "正在思考…"}
          </span>
          <span className="font-mono text-xs text-muted-foreground/70">{mm}:{ss}</span>
        </div>
        {elapsed >= 30 && (
          <p className="mt-1 text-[11px] text-muted-foreground/60">
            复杂问题可能需要较长时间 — 你可以切换页面，回复仍会送达当前会话
          </p>
        )}
      </div>
    </div>
  );
}

export function ChatView() {
  const { chatState, currentSessionId, turns, sending, status, error, load, send, newSession, selectSession } =
    useChatStore();
  const [input, setInput] = useState("");
  const [pendingAttachment, setPendingAttachment] = useState<ChatAttachment | null>(null);
  const [attaching, setAttaching] = useState(false);
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const imageInputRef = useRef<HTMLInputElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const [recordingSeconds, setRecordingSeconds] = useState(0);

  useEffect(() => {
    load();
  }, [load]);

  const currentSession = chatState?.sessions.find((s) => s.id === currentSessionId);

  // Switching sessions must not carry over the previous session's errors.
  useEffect(() => {
    setActionError(null);
  }, [currentSessionId]);

  // Auto-scroll to bottom on new messages.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [turns, status]);

  const handleSend = () => {
    if (sending) return;
    if (!input.trim() && !pendingAttachment) return;
    const paths = pendingAttachment?.path ? [pendingAttachment.path] : undefined;
    const attachments = pendingAttachment ? [pendingAttachment] : undefined;
    send(input.trim(), paths, attachments);
    setInput("");
    setPendingAttachment(null);
    if (imageInputRef.current) imageInputRef.current.value = "";
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Enter sends; Shift+Enter inserts a newline.
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  // ── Attachments (#4074): image (persisted to vault) or generic file ──────
  const handleFileChosen = async (file: File | undefined, isImage: boolean) => {
    setActionError(null);
    setAddOpen(false);
    if (!file) return;
    setAttaching(true);
    try {
      const dataUrl = await blobToDataUrl(file);
      // Images are persisted into the vault (attachments/chat/) so history
      // survives temp-dir wipes and chat_state stays free of base64 blobs
      // (#4083). The in-memory dataUrl is kept for optimistic rendering.
      const path = await api.saveTempAttachment(dataUrlToBase64(dataUrl), file.name, isImage);
      setPendingAttachment({ name: file.name, type: file.type, dataUrl, path });
    } catch (e) {
      setActionError(`${isImage ? "图片" : "文件"}保存失败：${String(e)}`);
    } finally {
      setAttaching(false);
    }
  };

  // ── Voice input: record → transcribe (STT) (#4074) ──────────────────────
  // The stream also feeds an AnalyserNode so the waveform canvas can show
  // live input levels — without that feedback the user can't tell whether
  // the mic is actually picking anything up (#4085).
  const teardownAnalyser = () => {
    analyserRef.current = null;
    const ctx = audioCtxRef.current;
    audioCtxRef.current = null;
    if (ctx && ctx.state !== "closed") void ctx.close().catch(() => {});
  };

  const stopRecording = () => {
    try {
      mediaRecorderRef.current?.stop();
    } catch {
      /* already stopped */
    }
    teardownAnalyser();
  };

  const startRecording = async () => {
    setActionError(null);
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
      setActionError("当前环境不支持录音（需要 WebView 麦克风权限）");
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      // Wire the level meter (separate from the recorder's stream usage; the
      // analyser is NOT connected to destination, so no echo/feedback).
      try {
        const audioCtx = new AudioContext();
        const source = audioCtx.createMediaStreamSource(stream);
        const analyser = audioCtx.createAnalyser();
        analyser.fftSize = 512;
        analyser.smoothingTimeConstant = 0.75;
        source.connect(analyser);
        audioCtxRef.current = audioCtx;
        analyserRef.current = analyser;
      } catch {
        /* waveform is cosmetic — recording proceeds without it */
      }
      const recorder = new MediaRecorder(stream);
      chunksRef.current = [];
      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) chunksRef.current.push(e.data);
      };
      recorder.onstop = () => {
        stream.getTracks().forEach((t) => t.stop());
        teardownAnalyser();
        void handleRecordingDone();
      };
      recorder.onerror = () => {
        stream.getTracks().forEach((t) => t.stop());
        teardownAnalyser();
        setRecording(false);
        setActionError("录音失败（麦克风被占用或权限被拒）");
      };
      mediaRecorderRef.current = recorder;
      recorder.start();
      setRecordingSeconds(0);
      setRecording(true);
    } catch (e) {
      setActionError(`无法访问麦克风：${String(e)}（请在系统设置中允许录音权限）`);
    }
  };

  // Elapsed-time counter while recording (interval, not rAF, to avoid
  // re-rendering the whole view per animation frame).
  useEffect(() => {
    if (!recording) return;
    const t = window.setInterval(() => setRecordingSeconds((s) => s + 1), 1000);
    return () => window.clearInterval(t);
  }, [recording]);

  const handleRecordingDone = async () => {
    setRecording(false);
    const chunks = chunksRef.current;
    if (chunks.length === 0) return;
    const blob = new Blob(chunks, { type: "audio/webm" });
    setTranscribing(true);
    try {
      const dataUrl = await blobToDataUrl(blob);
      const path = await api.saveTempAttachment(dataUrlToBase64(dataUrl), "voice.webm");
      const transcript = await api.transcribeAudio(path);
      setInput((prev) => (prev ? `${prev}\n${transcript}` : transcript));
    } catch (e) {
      setActionError(`语音转文字失败：${String(e)}`);
    } finally {
      setTranscribing(false);
    }
  };

  const toggleRecording = () => {
    if (recording) stopRecording();
    else void startRecording();
  };

  return (
    <div className="flex h-full flex-col">
      {/* Mobile-only session switcher (#4074): desktop uses the Sidebar. */}
      {chatState && chatState.sessions.length > 0 && (
        <div className="flex items-center gap-2 border-b border-border bg-card px-3 py-2 md:hidden">
          <select
            aria-label="切换对话"
            value={currentSessionId ?? ""}
            onChange={(e) => selectSession(e.target.value)}
            className="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1.5 text-sm"
          >
            {chatState.sessions.map((s) => (
              <option key={s.id} value={s.id}>
                {s.title || "新会话"}
              </option>
            ))}
          </select>
          <Button onClick={newSession} variant="ghost" size="icon" title="新会话">
            +
          </Button>
        </div>
      )}

      {/* Messages */}
      <ScrollArea className="flex-1">
        <div ref={scrollRef} className="mx-auto max-w-3xl py-4">
          {turns.length === 0 && !sending && (
            <div className="flex h-full min-h-[40vh] flex-col items-center justify-center text-center">
              <h2 className="text-2xl font-semibold tracking-tight">
                {currentSession?.title || "开始对话"}
              </h2>
              <p className="mt-2 text-sm text-muted-foreground">
                <span className="hidden md:inline">输入问题，Ctrl+Enter 发送</span>
                <span className="md:hidden">输入问题，点击发送</span>
              </p>
            </div>
          )}
          {turns.map((m, i) => (
            <MessageBubble key={m.id ?? i} turn={m} />
          ))}
          {sending && <ThinkingIndicator detail={status?.detail ?? null} />}
          {(error || actionError) && (
            <div className="mx-auto my-2 max-w-3xl px-4">
              <p className="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
                {actionError ?? error}
              </p>
            </div>
          )}
        </div>
      </ScrollArea>

      {/* Composer */}
      <div className="border-t border-border bg-background p-3">
        {/* Live recording feedback: elapsed time + real-time mic level so the
            user can see voice is actually being captured (#4085). */}
        {recording && (
          <div className="mx-auto mb-2 flex max-w-3xl items-center gap-3 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-1.5">
            <span className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-destructive" />
            <span className="shrink-0 font-mono text-xs text-destructive">
              {Math.floor(recordingSeconds / 60)}:{String(recordingSeconds % 60).padStart(2, "0")}
            </span>
            <Waveform analyserRef={analyserRef} active={recording} className="h-9 min-w-0 flex-1" />
            <span className="shrink-0 text-xs text-muted-foreground">点击 ⏹ 结束</span>
          </div>
        )}
        <div className="mx-auto flex max-w-3xl items-end gap-2">
          {pendingAttachment && (
            <div className="relative shrink-0">
              {pendingAttachment.type?.startsWith("image/") ? (
                /* eslint-disable-next-line @next/next/no-img-element */
                <img
                  src={pendingAttachment.dataUrl}
                  alt={pendingAttachment.name ?? "attachment"}
                  className="h-14 w-14 rounded-md border border-border object-cover"
                />
              ) : (
                <div className="flex h-14 max-w-40 items-center gap-2 rounded-md border border-border bg-card px-3 text-xs">
                  <FileIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="truncate">{pendingAttachment.name ?? "文件"}</span>
                </div>
              )}
              <button
                type="button"
                onClick={() => {
                  setPendingAttachment(null);
                  if (imageInputRef.current) imageInputRef.current.value = "";
                  if (fileInputRef.current) fileInputRef.current.value = "";
                }}
                aria-label="移除附件"
                className="absolute -right-1.5 -top-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-destructive text-[10px] leading-none text-white"
              >
                ×
              </button>
            </div>
          )}

          <input
            ref={imageInputRef}
            type="file"
            accept="image/*"
            className="hidden"
            onChange={(e) => void handleFileChosen(e.target.files?.[0], true)}
          />
          <input
            ref={fileInputRef}
            type="file"
            className="hidden"
            onChange={(e) => void handleFileChosen(e.target.files?.[0], false)}
          />

          {/* "+" → pick image or file (#二级菜单) */}
          <div className="relative shrink-0">
            <Button
              onClick={() => setAddOpen((v) => !v)}
              variant="ghost"
              size="icon"
              title="添加附件"
              disabled={sending || attaching || recording}
            >
              <PlusIcon className="h-5 w-5" />
            </Button>
            {addOpen && (
              <>
                <div className="fixed inset-0 z-10" onClick={() => setAddOpen(false)} />
                <div className="absolute bottom-11 left-0 z-20 flex w-36 flex-col overflow-hidden rounded-md border border-border bg-card shadow-lg">
                  <button
                    onClick={() => {
                      setAddOpen(false);
                      imageInputRef.current?.click();
                    }}
                    disabled={sending || attaching || recording}
                    className="flex items-center gap-2 px-3 py-2 text-left text-sm transition-colors hover:bg-accent disabled:opacity-50"
                  >
                    <ImageIcon className="h-4 w-4 text-muted-foreground" />
                    发送图片
                  </button>
                  <button
                    onClick={() => {
                      setAddOpen(false);
                      fileInputRef.current?.click();
                    }}
                    disabled={sending || attaching || recording}
                    className="flex items-center gap-2 border-t border-border px-3 py-2 text-left text-sm transition-colors hover:bg-accent disabled:opacity-50"
                  >
                    <FileIcon className="h-4 w-4 text-muted-foreground" />
                    发送文件
                  </button>
                </div>
              </>
            )}
          </div>

          <Textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="输入问题…（Ctrl+Enter 发送）"
            rows={2}
            className="min-h-[44px] flex-1 resize-none"
            disabled={sending}
          />
          <Button
            onClick={handleSend}
            disabled={sending || (!input.trim() && !pendingAttachment)}
            size="default"
          >
            发送
          </Button>
          {/* Mic on the right of send */}
          <Button
            onClick={toggleRecording}
            variant={recording ? "default" : "ghost"}
            size="icon"
            title={recording ? "停止录音" : "语音输入（转文字）"}
            disabled={sending || transcribing || attaching}
            className={cn(recording && "bg-destructive text-white")}
          >
            {transcribing ? (
              <span className="h-4 w-4 animate-spin rounded-full border-2 border-current border-t-transparent" />
            ) : recording ? (
              <StopIcon className="h-4 w-4" />
            ) : (
              <MicIcon className="h-5 w-5" />
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}
