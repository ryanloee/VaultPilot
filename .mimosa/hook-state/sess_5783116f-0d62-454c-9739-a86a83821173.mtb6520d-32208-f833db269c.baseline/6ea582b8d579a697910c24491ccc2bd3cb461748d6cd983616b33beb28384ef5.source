import { useEffect } from "react";
import { AppShell } from "@/components/layout/AppShell";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { ensureNotificationPermission } from "@/lib/notify";

export default function App() {
  // Ask for POST_NOTIFICATIONS once on mobile (Android 13+); no-op when
  // already granted or on desktop where toasts are allowed by default.
  useEffect(() => {
    void ensureNotificationPermission();
  }, []);

  return (
    <ErrorBoundary>
      <AppShell />
    </ErrorBoundary>
  );
}
