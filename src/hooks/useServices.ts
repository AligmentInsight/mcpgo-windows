// React hook for the services view: the joined definition + SCM state list
// and every action (CRUD plus the elevated install / uninstall / start /
// stop / restart operations).
//
// The SCM has no push channel (status changes are not observable without
// platform APIs), so state is PULLED: refresh runs on mount, after every
// successful action and whenever the user clicks Refresh in the view.
//
// Elevated actions block until their UAC-elevated PowerShell script
// finishes; `saving` stays true for that whole period and disables every
// button of the view (one management operation at a time - the SCM itself
// is not transactional).

import { useCallback, useEffect, useState } from "react";
import {
  createServiceDefinition,
  deleteServiceDefinition,
  installService,
  listServices,
  restartService,
  startService,
  stopService,
  uninstallService,
  updateServiceDefinition,
} from "../api/services";
import type { ServiceDefinitionInput, ServiceSummary } from "../types/service";

export interface UseServicesResult {
  /** Known definitions joined with SCM state, sorted by display name. */
  summaries: ServiceSummary[];
  /** True while the initial/refresh load is in flight. */
  loading: boolean;
  /** Load failure message (also set when the IPC bridge is unavailable). */
  loadError: string | null;
  /** Re-queries the definitions and the SCM states. */
  refresh: () => Promise<void>;
  /** Creates a definition; resolves true on success. */
  create: (input: ServiceDefinitionInput, id?: string) => Promise<boolean>;
  /** Updates a definition; resolves true on success. */
  update: (id: string, input: ServiceDefinitionInput) => Promise<boolean>;
  /** Deletes a definition; resolves true on success. */
  remove: (id: string) => Promise<boolean>;
  /** Installs the Windows service of a definition (UAC prompt). */
  install: (id: string) => Promise<boolean>;
  /** Stops and deletes the Windows service (UAC prompt). */
  uninstall: (id: string) => Promise<boolean>;
  /** Starts the installed service (UAC prompt). */
  start: (id: string) => Promise<boolean>;
  /** Stops the running service (UAC prompt). */
  stop: (id: string) => Promise<boolean>;
  /** Stops and starts the installed service (UAC prompt). */
  restart: (id: string) => Promise<boolean>;
  /** True while a mutating action is in flight. */
  saving: boolean;
  /** Failure message of the last mutating action. */
  actionError: string | null;
}

export function useServices(): UseServicesResult {
  const [summaries, setSummaries] = useState<ServiceSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      setSummaries(await listServices());
    } catch (reason: unknown) {
      // IPC failures (including a plain browser preview without the
      // Tauri bridge) surface here: the services view is a primary view,
      // an empty silent list would be misleading.
      setLoadError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Shared engine of every mutating action: one in-flight operation,
  // one error channel, one refresh on success.
  const act = useCallback(
    async (action: () => Promise<unknown>): Promise<boolean> => {
      setSaving(true);
      setActionError(null);
      try {
        await action();
        await refresh();
        return true;
      } catch (reason: unknown) {
        setActionError(String(reason));
        return false;
      } finally {
        setSaving(false);
      }
    },
    [refresh],
  );

  const create = useCallback(
    (input: ServiceDefinitionInput, id?: string): Promise<boolean> =>
      act(() => createServiceDefinition(input, id)),
    [act],
  );

  const update = useCallback(
    (id: string, input: ServiceDefinitionInput): Promise<boolean> =>
      act(() => updateServiceDefinition(id, input)),
    [act],
  );

  const remove = useCallback(
    (id: string): Promise<boolean> => act(() => deleteServiceDefinition(id)),
    [act],
  );

  const install = useCallback(
    (id: string): Promise<boolean> => act(() => installService(id)),
    [act],
  );

  const uninstall = useCallback(
    (id: string): Promise<boolean> => act(() => uninstallService(id)),
    [act],
  );

  const start = useCallback(
    (id: string): Promise<boolean> => act(() => startService(id)),
    [act],
  );

  const stop = useCallback(
    (id: string): Promise<boolean> => act(() => stopService(id)),
    [act],
  );

  const restart = useCallback(
    (id: string): Promise<boolean> => act(() => restartService(id)),
    [act],
  );

  return {
    summaries,
    loading,
    loadError,
    refresh,
    create,
    update,
    remove,
    install,
    uninstall,
    start,
    stop,
    restart,
    saving,
    actionError,
  };
}
