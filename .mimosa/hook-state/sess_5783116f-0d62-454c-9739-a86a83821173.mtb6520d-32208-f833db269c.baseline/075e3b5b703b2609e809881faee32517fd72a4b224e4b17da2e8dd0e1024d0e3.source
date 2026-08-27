import { useEffect, useRef } from "react";

/**
 * Live microphone waveform (bar-style level meter) for the voice input UI.
 *
 * Reads amplitude data from a Web Audio `AnalyserNode` on a requestAnimationFrame
 * loop and paints it into a canvas — so the user sees real-time feedback that
 * the mic is actually picking up sound (#4085).
 *
 * The analyser is owned by the recorder setup in ChatView; this component is
 * purely the painter and never touches the audio graph itself.
 */
export function Waveform({
  analyserRef,
  active,
  bars = 28,
  className,
}: {
  analyserRef: React.MutableRefObject<AnalyserNode | null>;
  active: boolean;
  bars?: number;
  className?: string;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    if (!active) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let raf = 0;
    let data = new Uint8Array(0);

    const paint = () => {
      raf = requestAnimationFrame(paint);
      const analyser = analyserRef.current;
      if (!analyser) return;
      if (data.length !== analyser.frequencyBinCount) {
        data = new Uint8Array(analyser.frequencyBinCount);
      }
      analyser.getByteFrequencyData(data);

      const { width, height } = canvas;
      ctx.clearRect(0, 0, width, height);

      // Sample the low-frequency band (voice lives there) across `bars`
      // buckets; higher buckets are noisier and less useful for feedback.
      const usable = Math.max(1, Math.floor(data.length * 0.4));
      const step = usable / bars;
      const barWidth = width / bars;
      for (let i = 0; i < bars; i++) {
        // Average a bucket, then exaggerate quiet input so even soft speech
        // produces visible movement (0–255 → 0.08–1.0 range).
        let sum = 0;
        const from = Math.floor(i * step);
        const to = Math.min(usable, Math.floor((i + 1) * step));
        for (let j = from; j < to; j++) sum += data[j];
        const avg = to > from ? sum / (to - from) : 0;
        const level = Math.min(1, Math.max(0.08, (avg / 255) * 1.8));

        const barHeight = Math.max(3, level * height);
        const x = i * barWidth + barWidth * 0.15;
        const w = barWidth * 0.7;
        const y = (height - barHeight) / 2;
        // Center-aligned bars: symmetric around the middle like chat-app
        // voice ripples; rounded caps keep it soft.
        ctx.fillStyle = "rgba(239, 68, 68, 0.85)";
        ctx.fillRect(x, y, w, barHeight);
      }
    };
    raf = requestAnimationFrame(paint);
    return () => cancelAnimationFrame(raf);
  }, [active, analyserRef, bars]);

  if (!active) return null;
  return (
    <canvas
      ref={canvasRef}
      className={className}
      width={280}
      height={36}
      aria-label="录音音量实时波形"
    />
  );
}
