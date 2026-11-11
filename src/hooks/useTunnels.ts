// React hook for the tunnels view: the joined definition + runtime state
// list, the machine profile, live runtime transitions through the
// tunnel://status event and every user action (CRUD, managed key, start,
// stop).
//
// State flow is the hybrid of the services and browser hooks: definitions
// are PULLED (refresh on mount, after every successful action and on the
// Refresh button), runtime states are PUSHED (every transition arrives as
// a tunnel://status event and merges into the matching summary).

import { useCallback, useEffect, useState } from "react";
import {
  createTunnelDefinition,
  deleteTunnelDefinition,
  fetchTunnelsProfile,
  listTunnels,
  onTunnelStatusEvent,
  setTunnelApiKey,
  startTunnel,
  stopTunnel,
  updateTunnelDefinition,
} from "../api/tunnels";
import type { TunnelDefinitionInput, TunnelSummary, TunnelsProfile } from "../types/tunnel";

export interface UseTunnelsResult {
  /** Known definitions joined with runtime state, sorted by display name. */
  summaries: TunnelSummary[];
  /** Machine profile info for the view header (null before the load). */
  profile: TunnelsProfile | null;
  /** True while the initial/refresh load is in flight. */
  loading: boolean;
  /** Load failure message (also set when the IPC bridge is unavailable). */
  loadError: string | null;
  /** Re-queries the definitions and the runtime states. */
  refresh: () => Promise<void>;
  /** Creates a definition; resolves true on success. */
  create: (input: TunnelDefinitionInput, id?: string) => Promise<boolean>;
  /** Updates a definition; resolves true on success. */
  update: (id: string, input: TunnelDefinitionInput) => Promise<boolean>;
  /** Deletes a definition; resolves true on success. */
  remove: (id: string) => Promise<boolean>;
  /** Stores the managed API key of a definition; resolves true on success. */
  setApiKey: (id: string, key: string) => Promise<boolean>;
  /** Starts the runtime of a definition; resolves true on success. */
  start: (id: string) => Promise<boolean>;
  /** Stops the running runtime of a definition; resolves true on success. */
  stop: (id: string) => Promise<boolean>;
  /** True while a mutating action is in flight. */
  acting: boolean;
  /** Failure message of the last mutating action. */
  actionError: string | null;
}

/** Merges one pushed status into the summary list (upsert by definition id). */
function mergeStatus(
  summaries: TunnelSummary[],
  definitionId: string,
  patch: (summary: TunnelSummary) => TunnelSummary,
): TunnelSummary[] {
  const index = summaries.findIndex(
    (summary) => summary.definition.id === definitionId,
  );
  if (index === -1) {
    return summaries; // a definition created after the last refresh
  }
  const next = [...summaries];
  next[index] = patch(next[index]);
  return next;
}

export function useTunnels(): UseTunnelsResult {
  const [summaries, setSummaries] = useState<TunnelSummary[]>([]);
  const [profile, setProfile] = useState<TunnelsProfile | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [acting, setActing] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Initial load plus the event subscription. The list load and the
  // profile load settle independently; "loading" ends when both did.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    const listLoad = listTunnels()
      .then((list) => {
        if (!cancelled) setSummaries(list);
      })
      .catch((reason: unknown) => {
        // IPC failures (including a plain browser preview without the
        // Tauri bridge) surface here: an empty silent list would be
        // misleading.
        if (!cancelled) setLoadError(String(reason));
      });

    const profileLoad = fetchTunnelsProfile()
      .then((info) => {
        if (!cancelled) setProfile(info);
      })
      .catch(() => {
        // Without the bridge there is no profile either; the header shows
        // its "not found" warning instead.
      });

    onTunnelStatusEvent((status) => {
      setSummaries((previous) =>
        mergeStatus(previous, status.definitionId, (summary) => ({
          ...summary,
          status,
        })),
      );
    })
      .then((unsubscribe) => {
        unlisten = unsubscribe;
      })
      .catch(() => {
        // Same as above: without the bridge there are no events either.
      });

    Promise.allSettled([listLoad, profileLoad]).finally(() => {
      if (!cancelled) setLoading(false);
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      setSummaries(await listTunnels());
    } catch (reason: unknown) {
      setLoadError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  // Shared engine of every mutating action: one in-flight operation, one
  // error channel, one refresh on success (the start action additionally
  // merges the returned "starting" summary immediately).
  const act = useCallback(
    async (action: () => Promise<unknown>): Promise<boolean> => {
      setActing(true);
      setActionError(null);
      try {
        await action();
        await refresh();
        return true;
      } catch (reason: unknown) {
        setActionError(String(reason));
        return false;
      } finally {
        setActing(false);
      }
    },
    [refresh],
  );

  const create = useCallback(
    (input: TunnelDefinitionInput, id?: string): Promise<boolean> =>
      act(() => createTunnelDefinition(input, id)),
    [act],
  );

  const update = useCallback(
    (id: string, input: TunnelDefinitionInput): Promise<boolean> =>
      act(() => updateTunnelDefinition(id, input)),
    [act],
  );

  const remove = useCallback(
    (id: string): Promise<boolean> => act(() => deleteTunnelDefinition(id)),
    [act],
  );

  const setApiKey = useCallback(
    (id: string, key: string): Promise<boolean> =>
      act(() => setTunnelApiKey(id, key)),
    [act],
  );

  const start = useCallback(
    async (id: string): Promise<boolean> => {
      setActing(true);
      setActionError(null);
      try {
        const summary = await startTunnel(id);
        // Merge the initial "starting" snapshot right away; the later
        // transitions arrive through the event subscription.
        setSummaries((previous) =>
          mergeStatus(previous, id, (existing) => ({
            ...existing,
            status: summary.status,
          })),
        );
        return true;
      } catch (reason: unknown) {
        setActionError(String(reason));
        return false;
      } finally {
        setActing(false);
      }
    },
    [],
  );

  const stop = useCallback(
    async (id: string): Promise<boolean> => {
      setActing(true);
      setActionError(null);
      try {
        const summary = await stopTunnel(id);
        setSummaries((previous) =>
          mergeStatus(previous, id, (existing) => ({
            ...existing,
            status: summary.status,
          })),
        );
        return true;
      } catch (reason: unknown) {
        setActionError(String(reason));
        return false;
      } finally {
        setActing(false);
      }
    },
    [],
  );

  return {
    summaries,
    profile,
    loading,
    loadError,
    refresh,
    create,
    update,
    remove,
    setApiKey,
    start,
    stop,
    acting,
    actionError,
  };
}
