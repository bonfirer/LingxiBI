import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EnvelopeSimple, PaperPlaneTilt } from '@phosphor-icons/react';
import { useAlertStore } from '../stores/alertStore';
import { alertsApi } from '../lib/api';

export default function SmtpModal({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const { smtp, fetchSmtp, updateSmtp } = useAlertStore();

  const [host, setHost] = useState('');
  const [port, setPort] = useState('465');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [fromEmail, setFromEmail] = useState('');
  const [fromName, setFromName] = useState('HISENSE LingxiBI');
  const [useTls, setUseTls] = useState(true);
  const [enabled, setEnabled] = useState(false);

  const [testTo, setTestTo] = useState('');
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ type: 'ok' | 'err'; text: string } | null>(null);

  useEffect(() => {
    fetchSmtp();
  }, [fetchSmtp]);

  useEffect(() => {
    if (smtp) {
      setHost(smtp.host);
      setPort(String(smtp.port));
      setUsername(smtp.username);
      setFromEmail(smtp.from_email);
      setFromName(smtp.from_name);
      setUseTls(smtp.use_tls);
      setEnabled(smtp.enabled);
    }
  }, [smtp]);

  const flash = (type: 'ok' | 'err', text: string) => {
    setMsg({ type, text });
    setTimeout(() => setMsg(null), 4000);
  };

  const handleSave = async () => {
    setBusy(true);
    try {
      await updateSmtp({
        host,
        port: parseInt(port) || 465,
        username,
        ...(password ? { password } : {}),
        from_email: fromEmail,
        from_name: fromName,
        use_tls: useTls,
        enabled,
      });
      setPassword('');
      flash('ok', t('alerts.saved'));
    } catch (e) {
      flash('err', (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const handleTest = async () => {
    if (!testTo.trim()) { flash('err', t('alerts.smtp.testToRequired')); return; }
    setBusy(true);
    try {
      // Persist current values first so the test uses them.
      await handleSave();
      const res = await alertsApi.testSmtp(testTo.trim());
      flash('ok', res.message);
    } catch (e) {
      flash('err', (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const labelCls = 'block text-[11px] font-medium text-gray-400 mb-1';
  const inputCls = 'w-full bg-obsidian-900 border border-obsidian-700 rounded-md px-2.5 py-1.5 text-xs text-gray-200 focus:border-amber-500/50 focus:outline-none';

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-[460px] shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-semibold text-gray-100 flex items-center gap-2">
            <EnvelopeSimple size={16} className="text-amber-500" />
            {t('alerts.smtpSettings')}
          </h3>
          {msg && (
            <span className={`text-[11px] px-2 py-1 rounded ${msg.type === 'ok' ? 'bg-green-500/10 text-green-400' : 'bg-red-500/10 text-red-400'}`}>
              {msg.text}
            </span>
          )}
        </div>

        <div className="space-y-3">
          <div className="grid grid-cols-3 gap-3">
            <div className="col-span-2">
              <label className={labelCls}>{t('alerts.smtp.host')}</label>
              <input className={inputCls} value={host} onChange={(e) => setHost(e.target.value)} placeholder="smtp.example.com" />
            </div>
            <div>
              <label className={labelCls}>{t('alerts.smtp.port')}</label>
              <input className={inputCls} type="number" value={port} onChange={(e) => setPort(e.target.value)} />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls}>{t('alerts.smtp.username')}</label>
              <input className={inputCls} value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" />
            </div>
            <div>
              <label className={labelCls}>{t('alerts.smtp.password')}</label>
              <input className={inputCls} type="password" value={password} onChange={(e) => setPassword(e.target.value)} placeholder={smtp?.password_set ? '••••••••' : ''} autoComplete="new-password" />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls}>{t('alerts.smtp.fromEmail')}</label>
              <input className={inputCls} value={fromEmail} onChange={(e) => setFromEmail(e.target.value)} placeholder="alerts@example.com" />
            </div>
            <div>
              <label className={labelCls}>{t('alerts.smtp.fromName')}</label>
              <input className={inputCls} value={fromName} onChange={(e) => setFromName(e.target.value)} />
            </div>
          </div>

          <div className="flex items-center gap-4">
            <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
              <input type="checkbox" checked={useTls} onChange={(e) => setUseTls(e.target.checked)} className="accent-amber-500" />
              {t('alerts.smtp.useTls')}
            </label>
            <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
              <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} className="accent-amber-500" />
              {t('alerts.smtp.enabled')}
            </label>
          </div>

          <div className="border-t border-obsidian-700 pt-3">
            <label className={labelCls}>{t('alerts.smtp.testEmail')}</label>
            <div className="flex items-center gap-2">
              <input className={`${inputCls} flex-1`} value={testTo} onChange={(e) => setTestTo(e.target.value)} placeholder="you@example.com" />
              <button onClick={handleTest} disabled={busy} className="px-3 py-1.5 rounded-md bg-obsidian-800 text-gray-200 text-xs hover:bg-obsidian-700 disabled:opacity-50 flex items-center gap-1.5 whitespace-nowrap">
                <PaperPlaneTilt size={12} />
                {t('common.test')}
              </button>
            </div>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 mt-5">
          <button onClick={onClose} className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700">
            {t('common.cancel')}
          </button>
          <button onClick={handleSave} disabled={busy} className="text-xs text-[#08080c] bg-amber-500 hover:bg-amber-400 px-4 py-1.5 rounded-md font-semibold disabled:opacity-50">
            {t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
}
