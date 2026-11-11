// React hook for the browser view: the session list, the machine profile,
// live updates through the browser://status event and the user actions
// (launch / stop / select).
//
// Sessions are NOT persisted by the backend (a debug-enabled Chrome is
// fully described by its loopback endpoint): the list on mount covers the
// endpoint-discovery results of this run, and every later transition
// arrives as an event.

import { useCallback, useEffect, useState } from "react";
import {
  fetchBrowserProfile,
  launchBrowser,
  listBrowserSessions,
  onBrowserStatusEvent,
  stopBrowserSession,
} from "../api/browser";
import type { BrowserLaunchOptions, BrowserProfile, BrowserSession } from "../types/browser";

export interface UseBrowserSessionsResult {
  /** Known sessions of this run, newest first. */
  sessions: BrowserSession[];
  /** Machine profile info for the view header (null before the load). */
  profile: BrowserProfile | null;
  /** True while the initial load is in flight. */
  loading: boolean;
  /** Load failure message (also set when the IPC bridge is unavailable). */
  loadError: string | null;
  /** Currently selected session id (drives the targets pane). */
  selectedId: string | null;
  select: (sessionId: string) => void;
  /** Launches a new browser session and selects it. */
  launch: (options: BrowserLaunchOptions) => Promise<void>;
  /** Requests the stop of one launched session. */
  stop: (sessionId: string) => Promise<void>;
  /** True while a launch or stop is in flight. */
  acting: boolean;
  /** Failure message of the last launch/stop action. */
  actionError: string | null;
}

/** Newest first; stable tie-breaking for ids created in the same millisecond. */
function sortSessions(list: BrowserSession[]): BrowserSession[] {
  return [...list].sort(
    (a, b) => b.createdAt - a.createdAt || b.id.localeCompare(a.id),
  );
}

/** Inserts or replaces a session, keeping the list sorted. */
function upsert(list: BrowserSession[], session: BrowserSession): BrowserSession[] {
  return sortSessions([
    ...list.filter((existing) => existing.id !== session.id),
    session,
  ]);
}

export function useBrowserSessions(): UseBrowserSessionsResult {
  const [sessions, setSessions] = useState<BrowserSession[]>([]);
  const [profile, setProfile] = useState<BrowserProfile | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [acting, setActing] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    const sessionsLoad = listBrowserSessions()
      .then((list) => {
        if (!cancelled) setSessions(sortSessions(list));
      })
      .catch((reason: unknown) => {
        // IPC failures (including a plain browser preview without the
        // Tauri bridge) surface here: an empty silent session list would
        // be misleading.
        if (!cancelled) setLoadError(String(reason));
      });

    const profileLoad = fetchBrowserProfile()
      .then((info) => {
        if (!cancelled) setProfile(info);
      })
      .catch(() => {
        // Without the bridge there is no profile either; the header shows
        // the unavailable state through its "not found" badge.
      });

    onBrowserStatusEvent((session) => {
      setSessions((previous) => upsert(previous, session));
    })
      .then((unsubscribe) => {
        unlisten = unsubscribe;
      })
      .catch(() => {
        // Same as above: without the bridge there are no events either.
      });

    // "Loading" ends as soon as both initial queries settled; the session
    // list stays live through the event subscription afterwards.
    Promise.allSettled([sessionsLoad, profileLoad]).finally(() => {
      if (!cancelled) setLoading(false);
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const launch = useCallback(async (options: BrowserLaunchOptions) => {
    setActing(true);
    setActionError(null);
    try {
      const session = await launchBrowser(options);
      setSessions((previous) => upsert(previous, session));
      setSelectedId(session.id);
    } catch (reason: unknown) {
      setActionError(String(reason));
    } finally {
      setActing(false);
    }
  }, []);

  const stop = useCallback(async (sessionId: string) => {
    setActing(true);
    setActionError(null);
    try {
      const session = await stopBrowserSession(sessionId);
      setSessions((previous) => upsert(previous, session));
    } catch (reason: unknown) {
      setActionError(String(reason));
    } finally {
      setActing(false);
    }
  }, []);

  return {
    sessions,
    profile,
    loading,
    loadError,
    selectedId,
    select: setSelectedId,
    launch,
    stop,
    acting,
    actionError,
  };
}
