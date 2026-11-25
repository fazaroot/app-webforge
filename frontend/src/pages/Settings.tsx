import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api, isNativeWebForge, openBatterySettings, resetApiConfigCache, resolveApiConfig } from '../lib/api';
import { Card, ErrorBox, btnGhost, btnPrimary, inputCls } from '../components/ui';
import { useAppStore, type ThemeMode } from '../store';

export function Settings() {
  const qc = useQueryClient();
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const [apiBase, setApiBase] = useState(localStorage.getItem('webforge_api_base') || 'http://127.0.0.1:8765');
  const [token, setToken] = useState(localStorage.getItem('webforge_token') || '');
  const [lanDefault, setLanDefault] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [testing, setTesting] = useState(false);
  const native = isNativeWebForge();

  const settings = useQuery({ queryKey: ['settings'], queryFn: api.settings });

  useEffect(() => {
    if (settings.data) {
      if (typeof settings.data.lan_default === 'boolean') setLanDefault(settings.data.lan_default);
      const t = settings.data.theme;
      if (t === 'light' || t === 'dark' || t === 'system') setTheme(t);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.data]);

  useEffect(() => {
    void resolveApiConfig().then((c) => { setApiBase(c.base); }).catch(() => undefined);
  }, []);

  const save = useMutation({
    mutationFn: () => api.putSettings({ theme, lan_default: lanDefault }),
    onSuccess: () => { setNotice('Pengaturan tersimpan di Core.'); setError(''); void qc.invalidateQueries({ queryKey: ['settings'] }); },
    onError: (e) => setError(e instanceof Error ? e.message : 'Gagal menyimpan pengaturan'),
  });

  function saveDevConnection() {
    localStorage.setItem('webforge_api_base', apiBase.trim());
    localStorage.setItem('webforge_token', token.trim());
    resetApiConfigCache();
    setNotice('Koneksi dev tersimpan. Menguji ulang…');
    void testConnection();
  }

  async function handleOpenBatterySettings() {
    try {
      await openBatterySettings();
      setNotice('Pengaturan baterai dibuka. Izinkan WebForge Mobile berjalan di latar belakang agar server tidak dihentikan Android.');
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Gagal membuka pengaturan baterai');
    }
  }

  async function testConnection() {
    setTesting(true);
    try {
      resetApiConfigCache();
      const h = await api.health();
      setNotice(`Terhubung ke Core v${h.version} (status ${h.status}).`);
      setError('');
      void qc.invalidateQueries();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Tidak dapat terhubung ke Core');
    } finally {
      setTesting(false);
    }
  }

  return (
    <div className="space-y-4">
      {error && <ErrorBox message={error} />}
      {notice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2">{notice}</div>}

      <Card title="Tampilan">
        <label className="block text-sm">
          <span className="text-xs font-medium text-slate-500">Tema</span>
          <select className={inputCls} value={theme} onChange={(e) => setTheme(e.target.value as ThemeMode)}>
            <option value="system">Ikuti sistem</option>
            <option value="light">Terang</option>
            <option value="dark">Gelap</option>
          </select>
        </label>
        <label className="flex items-start gap-2 text-sm mt-3">
          <input type="checkbox" className="mt-0.5" checked={lanDefault} onChange={(e) => setLanDefault(e.target.checked)} />
          <span>Server baru secara default membuka akses LAN</span>
        </label>
        {lanDefault && <p className="text-xs text-warning mt-1">Perhatian: default LAN membuat server baru dapat diakses perangkat lain di jaringan yang sama.</p>}
        <button type="button" className={`${btnPrimary} mt-3`} disabled={save.isPending} onClick={() => save.mutate()}>
          {save.isPending ? 'Menyimpan…' : 'Simpan ke Core'}
        </button>
        {settings.isError && <p className="text-xs text-danger mt-2">Pengaturan Core belum dapat dimuat (Core offline?). Tema lokal tetap berlaku.</p>}
      </Card>

      {native && (
        <Card title="Latar belakang & baterai">
          <p className="text-xs text-slate-500 mb-2">
            Server berjalan lewat layanan latar depan Android. Sebagian perangkat menghentikan aplikasi
            di latar belakang demi menghemat baterai; beri pengecualian optimasi baterai agar server
            tidak mati saat layar mati.
          </p>
          <button type="button" className={btnGhost} onClick={() => void handleOpenBatterySettings()}>
            Buka Pengaturan Baterai
          </button>
        </Card>
      )}

      <Card title="Koneksi Core (pengembangan)">
        <p className="text-xs text-slate-500 mb-2">
          Di aplikasi Android, alamat dan token diambil otomatis dari Core lewat plugin native.
          Isian ini hanya untuk pengembangan di browser.
        </p>
        <div className="space-y-2">
          <input className={inputCls} placeholder="API base, mis. http://127.0.0.1:8765" value={apiBase} onChange={(e) => setApiBase(e.target.value)} />
          <input className={inputCls} type="password" placeholder="Token API (dev)" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" />
          <div className="flex gap-2">
            <button type="button" className={btnPrimary} onClick={saveDevConnection}>Simpan koneksi</button>
            <button type="button" className={btnGhost} disabled={testing} onClick={() => void testConnection()}>
              {testing ? 'Menguji…' : 'Uji koneksi'}
            </button>
          </div>
        </div>
      </Card>
    </div>
  );
}
