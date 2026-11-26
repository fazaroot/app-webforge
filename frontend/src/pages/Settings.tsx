import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  api, getPowerStatus, isNativeWebForge, openBatterySettings, requestIgnoreBatteryOptimizations,
  resetApiConfigCache, resolveApiConfig, setKeepCpuAwake,
} from '../lib/api';
import type { MailSecurity, MailTestResult, PowerStatus } from '../lib/types';
import { Card, ErrorBox, btnGhost, btnPrimary, inputCls } from '../components/ui';
import { useAppStore, type ThemeMode } from '../store';

function MailSettingsCard() {
  const qc = useQueryClient();
  const [host, setHost] = useState('');
  const [port, setPort] = useState('587');
  const [security, setSecurity] = useState<MailSecurity>('starttls');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [clearPassword, setClearPassword] = useState(false);
  const [from, setFrom] = useState('');
  const [testTo, setTestTo] = useState('');
  const [testResult, setTestResult] = useState<MailTestResult | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');

  const mail = useQuery({ queryKey: ['mail-settings'], queryFn: api.mailSettings });

  useEffect(() => {
    if (mail.data) {
      setHost(mail.data.host);
      setPort(String(mail.data.port));
      setSecurity(mail.data.security);
      setUsername(mail.data.username);
      setFrom(mail.data.from);
      setPassword('');
      setClearPassword(false);
    }
  }, [mail.data]);

  const save = useMutation({
    mutationFn: () => api.putMailSettings({
      host: host.trim(),
      port: Number(port),
      security,
      username: username.trim(),
      from: from.trim(),
      ...(password ? { password } : {}),
      ...(clearPassword ? { clear_password: true } : {}),
    }),
    onSuccess: () => {
      setNotice('Pengaturan SMTP tersimpan.');
      setError('');
      setPassword('');
      setClearPassword(false);
      void qc.invalidateQueries({ queryKey: ['mail-settings'] });
    },
    onError: (e) => { setNotice(''); setError(e instanceof Error ? e.message : 'Gagal menyimpan pengaturan SMTP'); },
  });

  const test = useMutation({
    mutationFn: () => api.testMail(testTo.trim()),
    onSuccess: (r) => { setTestResult(r); setError(''); },
    onError: (e) => { setTestResult(null); setError(e instanceof Error ? e.message : 'Kirim email tes gagal'); },
  });

  const portNum = Number(port);
  const portValid = Number.isInteger(portNum) && portNum >= 1 && portNum <= 65535;
  const saveReason = save.isPending
    ? 'Sedang menyimpan…'
    : !host.trim()
      ? 'Isi host SMTP dulu.'
      : !portValid
        ? 'Port SMTP tidak valid: isi bilangan 1–65535.'
        : !from.trim()
          ? 'Isi alamat pengirim (From) dulu.'
          : '';
  const testReason = test.isPending
    ? 'Sedang mengirim email tes…'
    : !testTo.trim()
      ? 'Isi alamat email tujuan dulu.'
      : !testTo.includes('@')
        ? 'Alamat email tujuan tidak valid.'
        : '';

  return (
    <Card title="Email (SMTP)">
      <div className="space-y-2">
        {error && <ErrorBox message={error} />}
        {notice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2">{notice}</div>}
        {mail.isLoading ? <p className="text-sm text-slate-700 dark:text-slate-300">Memuat pengaturan SMTP…</p> : mail.isError ? (
          <ErrorBox message={mail.error instanceof Error ? mail.error.message : 'Gagal memuat pengaturan SMTP.'} />
        ) : (
          <>
            <p className="text-sm">
              Status: {mail.data?.configured ? 'SMTP terkonfigurasi' : 'SMTP belum terkonfigurasi'}
              {mail.data?.has_password ? ' · password tersimpan' : ' · password belum tersimpan'}
            </p>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Host SMTP</span>
              <input className={inputCls} placeholder="mis. smtp.contoh.com" value={host} onChange={(e) => setHost(e.target.value)} />
            </label>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Port SMTP</span>
              <input className={inputCls} inputMode="numeric" placeholder="mis. 587" value={port} onChange={(e) => setPort(e.target.value)} />
            </label>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Keamanan koneksi</span>
              <select className={inputCls} value={security} onChange={(e) => setSecurity(e.target.value as MailSecurity)}>
                <option value="none">Tanpa enkripsi</option>
                <option value="starttls">STARTTLS</option>
                <option value="tls">TLS langsung</option>
              </select>
            </label>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Username SMTP</span>
              <input className={inputCls} placeholder="mis. user@contoh.com" value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" />
            </label>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Password SMTP</span>
              <input className={inputCls} type="password" autoComplete="off"
                placeholder={mail.data?.has_password ? 'Kosongkan untuk mempertahankan password tersimpan' : 'Password SMTP'}
                value={password} onChange={(e) => setPassword(e.target.value)} />
            </label>
            {mail.data?.has_password && (
              <label className="flex items-start gap-2 text-sm">
                <input type="checkbox" className="mt-0.5" checked={clearPassword} onChange={(e) => setClearPassword(e.target.checked)} />
                <span>Hapus password tersimpan</span>
              </label>
            )}
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Alamat pengirim (From)</span>
              <input className={inputCls} placeholder="mis. WebForge <noreply@contoh.com>" value={from} onChange={(e) => setFrom(e.target.value)} />
            </label>
            <button type="button" className={btnPrimary} disabled={!!saveReason} onClick={() => save.mutate()}>
              {save.isPending ? 'Menyimpan…' : 'Simpan SMTP'}
            </button>
            {saveReason && <p className="text-xs text-slate-800 dark:text-slate-200" role="status">Tombol nonaktif: {saveReason}</p>}
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Tujuan email tes</span>
              <input className={inputCls} placeholder="mis. kamu@contoh.com" value={testTo} onChange={(e) => setTestTo(e.target.value)} />
            </label>
            <button type="button" className={btnGhost} disabled={!!testReason} onClick={() => test.mutate()}>
              {test.isPending ? 'Mengirim…' : 'Kirim Email Tes'}
            </button>
            {testReason && <p className="text-xs text-slate-800 dark:text-slate-200" role="status">Tombol nonaktif: {testReason}</p>}
            {testResult && (
              <div className={`rounded-lg border px-3 py-2 text-sm ${testResult.ok ? 'bg-success/10 border-success/40' : 'bg-red-50/90 dark:bg-red-950/70 border-danger/50'}`}>
                <p className="font-medium">{testResult.ok ? 'Email tes terkirim.' : 'Email tes gagal.'} ({testResult.duration_ms} ms)</p>
                {testResult.error && <p className="text-danger break-all">{testResult.error}</p>}
                {testResult.output && <pre className="text-xs font-mono whitespace-pre-wrap break-all mt-1">{testResult.output}</pre>}
              </div>
            )}
            <p className="text-xs text-slate-700 dark:text-slate-300">Password hanya dikirim saat diisi dan tidak pernah ditampilkan kembali. Aplikasi PHP mengirim email lewat shim sendmail yang meneruskan ke SMTP ini.</p>
          </>
        )}
      </div>
    </Card>
  );
}

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
  const [power, setPower] = useState<PowerStatus | null>(null);
  const [powerBusy, setPowerBusy] = useState(false);
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

  useEffect(() => {
    if (!native) return;
    void getPowerStatus().then(setPower).catch((e: unknown) => {
      setError(e instanceof Error ? e.message : 'Gagal memuat status daya');
    });
  }, [native]);

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

  async function handleToggleKeepCpuAwake(enabled: boolean) {
    setPowerBusy(true);
    try {
      const r = await setKeepCpuAwake(enabled);
      setPower((p) => (p ? { ...p, keepCpuAwake: r.keepCpuAwake } : { ignoringBatteryOptimizations: false, keepCpuAwake: r.keepCpuAwake }));
      setNotice(r.keepCpuAwake
        ? 'WakeLock CPU aktif: CPU dijaga tetap bangun selama server berjalan.'
        : 'WakeLock CPU nonaktif: server bisa tertunda saat layar mati.');
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Gagal mengubah WakeLock CPU');
    } finally {
      setPowerBusy(false);
    }
  }

  async function handleRequestBatteryExemption() {
    setPowerBusy(true);
    try {
      const r = await requestIgnoreBatteryOptimizations();
      setPower((p) => (p ? { ...p, ignoringBatteryOptimizations: r.ignoringBatteryOptimizations } : { ignoringBatteryOptimizations: r.ignoringBatteryOptimizations, keepCpuAwake: false }));
      setNotice(r.ignoringBatteryOptimizations
        ? 'Pengecualian optimasi baterai aktif untuk WebForge Mobile.'
        : 'Dialog pengecualian dibuka. Bila kamu baru menyetujuinya, tekan "Perbarui status daya" untuk melihat status terbaru.');
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Gagal meminta pengecualian optimasi baterai');
    } finally {
      setPowerBusy(false);
    }
  }

  async function handleRefreshPowerStatus() {
    setPowerBusy(true);
    try {
      setPower(await getPowerStatus());
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Gagal memuat status daya');
    } finally {
      setPowerBusy(false);
    }
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
          <span className="text-xs font-medium text-slate-600 dark:text-slate-400">Tema</span>
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

      <Card title="Latar belakang & baterai">
        {!native ? (
          <p className="text-sm text-slate-600 dark:text-slate-400">
            Pengaturan anti-stuck (WakeLock CPU dan pengecualian optimasi baterai) hanya tersedia di
            aplikasi Android. Di browser, server tidak berjalan di perangkat ini sehingga pengaturan
            ini tidak berlaku.
          </p>
        ) : (
          <div className="space-y-3">
            <p className="text-xs text-slate-600 dark:text-slate-400">
              Server berjalan lewat layanan latar depan Android. Saat layar mati, CPU perangkat bisa
              tidur sehingga server terasa stuck sampai aplikasi dibuka lagi. WakeLock menjaga CPU
              tetap bangun selama server berjalan, dan pengecualian optimasi baterai mencegah Android
              menghentikan aplikasi di latar belakang.
            </p>
            <label className="flex items-start gap-2 text-sm">
              <input type="checkbox" className="mt-0.5" checked={power?.keepCpuAwake ?? false} disabled={powerBusy || power === null}
                onChange={(e) => void handleToggleKeepCpuAwake(e.target.checked)} />
              <span>Jaga CPU tetap bangun saat server berjalan</span>
            </label>
            <p className="text-sm">
              Pengecualian optimasi baterai:{' '}
              {power === null ? 'memuat…' : power.ignoringBatteryOptimizations
                ? <span className="text-success font-medium">aktif</span>
                : <span className="text-warning font-medium">belum aktif</span>}
            </p>
            <div className="flex flex-wrap gap-2">
              <button type="button" className={btnGhost} disabled={powerBusy} onClick={() => void handleRequestBatteryExemption()}>
                Minta pengecualian optimasi baterai
              </button>
              <button type="button" className={btnGhost} onClick={() => void handleOpenBatterySettings()}>
                Buka Pengaturan Baterai
              </button>
              <button type="button" className={btnGhost} disabled={powerBusy} onClick={() => void handleRefreshPowerStatus()}>
                Perbarui status daya
              </button>
            </div>
            <div className="rounded-lg bg-warning/10 border border-warning/40 text-sm px-3 py-2" role="alert">
              <strong>Panduan HyperOS (Xiaomi/POCO):</strong> buka Pengaturan → Aplikasi → WebForge Mobile,
              aktifkan <strong>Autostart</strong>, lalu di Penghemat baterai pilih <strong>Tanpa batasan</strong>.
              Langkah ini harus dilakukan manual — aplikasi tidak bisa mengubah Autostart atau mode
              baterai HyperOS sendiri.
            </div>
          </div>
        )}
      </Card>

      <MailSettingsCard />

      <Card title="Tentang">
        <div className="text-sm space-y-1">
          <p><strong>WebForge Mobile</strong> v0.5.0</p>
          <p>Brand: <strong>Faza X TCP</strong></p>
          <p className="text-slate-700 dark:text-slate-300">Lingkungan server web dan pengembangan lokal di Android: server, runtime, database, terminal, dan file manager berjalan nyata di perangkat ini.</p>
        </div>
      </Card>

      <Card title="Koneksi Core (pengembangan)">
        <p className="text-xs text-slate-600 dark:text-slate-400 mb-2">
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
