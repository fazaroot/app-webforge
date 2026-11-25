import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { api, getCoreStatus, isNativeWebForge, resetApiConfigCache, startCore } from './api';
import { runCoreBootstrap } from './bootstrap';

export type CoreBootstrapPhase = 'starting' | 'ready' | 'failed' | 'web';

export interface CoreBootstrapState {
  phase: CoreBootstrapPhase;
  /** loadError nyata dari plugin bila bootstrap gagal. */
  loadError: string | null;
  /** Mengulang bootstrap penuh (startCore + polling backoff). */
  retry: () => void;
}

/**
 * Di aplikasi native: menyalakan core lalu polling sampai sehat.
 * Di web non-native: fase 'web', perilaku health query lama dipertahankan.
 */
export function useCoreBootstrap(): CoreBootstrapState {
  const qc = useQueryClient();
  const [native] = useState(() => isNativeWebForge());
  const [phase, setPhase] = useState<CoreBootstrapPhase>(native ? 'starting' : 'web');
  const [loadError, setLoadError] = useState<string | null>(null);
  const [runId, setRunId] = useState(0);

  useEffect(() => {
    if (!native) return;
    let cancelled = false;
    setPhase('starting');
    void runCoreBootstrap({
      startCore,
      health: api.health,
      getStatus: async () => {
        try {
          return await getCoreStatus();
        } catch {
          return null;
        }
      },
      resetConfig: resetApiConfigCache,
    }).then((result) => {
      if (cancelled) return;
      if (result.ok) {
        setPhase('ready');
        setLoadError(null);
        void qc.invalidateQueries({ queryKey: ['health'] });
      } else {
        setPhase('failed');
        setLoadError(result.error);
      }
    });
    return () => { cancelled = true; };
  }, [native, runId, qc]);

  return { phase, loadError, retry: () => setRunId((x) => x + 1) };
}
