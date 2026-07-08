import { useEffect, useState, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { FloppyDisk, Lightning, Check, X, CaretDown, Sliders, Users, Plus, Trash } from '@phosphor-icons/react';
import { llmConfigApi, usersApi, type LLMConfig, type UpdateLLMConfigPayload, type User } from '../lib/api';
import { getCurrentUser, isAdmin } from '../lib/currentUser';
import { Select } from '../components/ui';

// ── Provider presets ──

interface ProviderPreset {
  label: string;
  base_url: string;
  models: string[];
}

const PROVIDERS: Record<string, ProviderPreset> = {
  openai: {
    label: 'OpenAI',
    base_url: 'https://api.openai.com/v1',
    models: ['gpt-4o', 'gpt-4o-mini', 'gpt-4-turbo', 'gpt-3.5-turbo', 'o1', 'o3-mini'],
  },
  deepseek: {
    label: 'DeepSeek',
    base_url: 'https://api.deepseek.com/v1',
    models: ['deepseek-v4-pro', 'deepseek-v4-flash'],
  },
  anthropic: {
    label: 'Anthropic',
    base_url: 'https://api.anthropic.com/v1',
    models: [
      'claude-sonnet-4-20250514',
      'claude-3-5-sonnet-20241022',
      'claude-3-5-haiku-20241022',
      'claude-3-opus-20240229',
    ],
  },
  hunyuan: {
    label: '腾讯混元',
    base_url: 'https://api.hunyuan.cloud.tencent.com/v1',
    models: [
      'hunyuan-turbos-latest',
      'hunyuan-pro',
      'hunyuan-standard',
      'hunyuan-lite',
      'hunyuan-vision',
    ],
  },
  zhipu: {
    label: '智谱 AI',
    base_url: 'https://open.bigmodel.cn/api/paas/v4',
    models: ['glm-4-plus', 'glm-4', 'glm-4-flash', 'glm-4-long'],
  },
  alibaba: {
    label: '阿里通义',
    base_url: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    models: ['qwen-turbo', 'qwen-plus', 'qwen-max', 'qwen-long', 'qwen-vl-plus'],
  },
  ollama: {
    label: 'Ollama (local)',
    base_url: 'http://localhost:11434/v1',
    models: ['llama3.2', 'qwen2.5', 'deepseek-r1:7b', 'mistral', 'codellama'],
  },
  custom: {
    label: 'Custom',
    base_url: '',
    models: [],
  },
};

export default function SettingsPage() {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [apiKeySet, setApiKeySet] = useState(false);
  const [config, setConfig] = useState<UpdateLLMConfigPayload>({
    provider: 'openai',
    base_url: 'https://api.openai.com/v1',
    api_key: '',
    model: 'gpt-4o',
    max_tokens: 4096,
    temperature: 0.1,
  });

  const [modelCustom, setModelCustom] = useState('');
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Models available for the current provider
  const availableModels = useMemo(
    () => PROVIDERS[config.provider ?? 'custom']?.models ?? [],
    [config.provider]
  );
  const isCustomModel =
    config.provider === 'custom' ||
    modelCustom !== '' ||
    (availableModels.length > 0 && !availableModels.includes(config.model ?? ''));

  const handleProviderChange = (provider: string) => {
    const preset = PROVIDERS[provider];
    if (preset) {
      const firstModel = preset.models[0] ?? '';
      setConfig((c) => ({
        ...c,
        provider,
        base_url: preset.base_url,
        model: firstModel,
      }));
      setModelCustom('');
    } else {
      setConfig((c) => ({ ...c, provider }));
    }
  };

  const handleModelSelect = (model: string) => {
    if (model === '__custom__') {
      setModelCustom(config.model ?? '');
    } else {
      setConfig((c) => ({ ...c, model }));
      setModelCustom('');
    }
  };

  const [testResult, setTestResult] = useState<{
    status: string;
    message: string;
  } | null>(null);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);

  useEffect(() => {
    llmConfigApi
      .get()
      .then((c: LLMConfig & { api_key_set?: boolean }) => {
        setApiKeySet(!!c.api_key_set);
        setConfig({
          provider: c.provider,
          base_url: c.base_url,
          // Never store the masked key in the form — leave empty.
          // On save, an empty key means "keep existing" (handled server-side).
          api_key: '',
          model: c.model,
          max_tokens: c.max_tokens,
          temperature: c.temperature,
        });
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const handleSave = async () => {
    try {
      setSaving(true);
      setSaveMsg(null);
      const updated = await llmConfigApi.update(config) as LLMConfig & { api_key_set?: boolean };
      // If we just saved a new key, reflect that and clear the input
      if (config.api_key) {
        setApiKeySet(!!updated.api_key_set);
        setConfig((c) => ({ ...c, api_key: '' }));
      }
      setSaveMsg(t('settings.saved'));
      setTimeout(() => setSaveMsg(null), 2000);
    } catch (e) {
      setSaveMsg(e instanceof Error ? e.message : t('errors.saveFailed'));
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async () => {
    try {
      setTesting(true);
      setTestResult(null);
      await llmConfigApi.update(config);
      const result = await llmConfigApi.test();
      setTestResult(result);
    } catch (e) {
      setTestResult({
        status: 'error',
        message: e instanceof Error ? e.message : t('errors.testFailed'),
      });
    } finally {
      setTesting(false);
    }
  };

  if (loading) {
    return (
      <div className="p-4 md:p-6 max-w-2xl">
        <div className="animate-pulse space-y-4">
          <div className="h-5 bg-obsidian-800 rounded w-32" />
          <div className="h-48 bg-obsidian-800 rounded-xl" />
        </div>
      </div>
    );
  }

  const admin = isAdmin();

  return (
    <div className="p-4 md:p-6 max-w-2xl">
      <div>
        <h1 className="text-lg font-bold text-gray-100 tracking-tight">
          {t('settings.title')}
        </h1>
        <p className="text-xs text-gray-500 mt-1">{t('settings.description')}</p>
      </div>

      <div className="mt-6 space-y-5">
        {!admin && (
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 text-xs text-gray-400">
            {t('settings.memberNotice')}
          </div>
        )}

        {admin && (
        <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5">
          <h2 className="text-sm font-semibold text-gray-200 mb-4">
            {t('settings.llmProvider')}
          </h2>

          <div className="space-y-1.5 mb-4">
            <label className="text-[11px] font-medium text-gray-400">
              {t('settings.provider')}
            </label>
            <Select
              value={config.provider}
              onChange={(v) => handleProviderChange(v)}
            >
              <option value="openai">{t('settings.providerOpenai')}</option>
              <option value="deepseek">{t('settings.providerDeepseek')}</option>
              <option value="anthropic">{t('settings.providerAnthropic')}</option>
              <option value="hunyuan">{t('settings.providerHunyuan')}</option>
              <option value="zhipu">{t('settings.providerZhipu')}</option>
              <option value="alibaba">{t('settings.providerAlibaba')}</option>
              <option value="ollama">{t('settings.providerOllama')}</option>
              <option value="custom">{t('settings.providerCustom')}</option>
            </Select>
          </div>

          <div className="space-y-1.5 mb-4">
            <label className="text-[11px] font-medium text-gray-400">
              {t('settings.baseUrl')}
            </label>
            <input
              type="text"
              value={config.base_url}
              onChange={(e) => setConfig({ ...config, base_url: e.target.value })}
              className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 font-mono placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
            />
            <p className="text-[10px] text-gray-600">{t('settings.baseUrlHint')}</p>
          </div>

          <div className="space-y-1.5 mb-4">
            <label className="text-[11px] font-medium text-gray-400">
              {t('settings.apiKey')}
            </label>
            <input
              type="password"
              value={config.api_key}
              onChange={(e) => setConfig({ ...config, api_key: e.target.value })}
              placeholder={apiKeySet ? t('settings.apiKeyConfigured') : t('settings.apiKeyPlaceholder')}
              className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 font-mono placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
            />
            {apiKeySet && (
              <p className="text-[9px] text-gray-600">{t('settings.apiKeyKeepHint')}</p>
            )}
          </div>

          <div className="space-y-1.5 mb-5">
            <label className="text-[11px] font-medium text-gray-400">
              {t('settings.model')}
            </label>

            {/* Dropdown for preset models */}
            {availableModels.length > 0 && !isCustomModel && (
              <Select
                value={config.model}
                onChange={(v) => handleModelSelect(v)}
                className="w-full font-mono"
              >
                {availableModels.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
                <option value="__custom__" className="text-amber-500">
                  — {t('settings.customModel')} —
                </option>
              </Select>
            )}

            {/* Text input for custom model */}
            {(isCustomModel || availableModels.length === 0) && (
              <div className="flex gap-2">
                <input
                  type="text"
                  value={modelCustom || config.model}
                  onChange={(e) => {
                    setModelCustom(e.target.value);
                    setConfig((c) => ({ ...c, model: e.target.value }));
                  }}
                  placeholder="e.g. my-fine-tuned-model"
                  className="flex-1 bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 font-mono placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
                />
                {availableModels.length > 0 && (
                  <button
                    onClick={() => {
                      setConfig((c) => ({ ...c, model: availableModels[0] }));
                      setModelCustom('');
                    }}
                    className="text-[10px] text-amber-500 hover:text-amber-400 px-2 py-1 rounded-md hover:bg-amber-500/10 transition-premium whitespace-nowrap"
                  >
                    {t('settings.usePreset')}
                  </button>
                )}
              </div>
            )}
          </div>

          {/* Advanced Settings Toggle */}
          <button
            onClick={() => setShowAdvanced(!showAdvanced)}
            className="flex items-center gap-1.5 text-[11px] text-gray-500 hover:text-gray-300 mb-4 transition-premium"
          >
            <Sliders size={12} />
            {t('settings.advancedSettings')}
            <CaretDown
              size={10}
              className={`transition-transform ${showAdvanced ? 'rotate-180' : ''}`}
            />
          </button>

          {showAdvanced && (
            <div className="space-y-4 mb-5 pl-3 border-l-2 border-obsidian-700">
              {/* Max Tokens */}
              <div className="space-y-1.5">
                <div className="flex items-center justify-between">
                  <label className="text-[11px] font-medium text-gray-400">
                    {t('settings.maxTokens')}
                  </label>
                  <span className="text-[10px] text-amber-500 font-mono">
                    {config.max_tokens?.toLocaleString()}
                  </span>
                </div>
                <input
                  type="range"
                  min={256}
                  max={32768}
                  step={256}
                  value={config.max_tokens ?? 4096}
                  onChange={(e) =>
                    setConfig({ ...config, max_tokens: parseInt(e.target.value) })
                  }
                  className="w-full h-1 bg-obsidian-700 rounded-lg appearance-none cursor-pointer accent-amber-500"
                />
                <div className="flex justify-between text-[9px] text-gray-600">
                  <span>256</span>
                  <span>32,768</span>
                </div>
              </div>

              {/* Temperature */}
              <div className="space-y-1.5">
                <div className="flex items-center justify-between">
                  <label className="text-[11px] font-medium text-gray-400">
                    {t('settings.temperature')}
                  </label>
                  <span className="text-[10px] text-amber-500 font-mono">
                    {(config.temperature ?? 0.1).toFixed(2)}
                  </span>
                </div>
                <input
                  type="range"
                  min={0}
                  max={2}
                  step={0.05}
                  value={config.temperature ?? 0.1}
                  onChange={(e) =>
                    setConfig({ ...config, temperature: parseFloat(e.target.value) })
                  }
                  className="w-full h-1 bg-obsidian-700 rounded-lg appearance-none cursor-pointer accent-amber-500"
                />
                <div className="flex justify-between text-[9px] text-gray-600">
                  <span>{t('settings.precise')}</span>
                  <span>{t('settings.creative')}</span>
                </div>
              </div>
            </div>
          )}

          {/* Test Result */}
          {testResult && (
            <div
              className={`flex items-center gap-2 text-xs px-3 py-2 rounded-lg mb-4 ${
                testResult.status === 'connected'
                  ? 'bg-data-green/10 text-data-green border border-data-green/20'
                  : 'bg-red-500/10 text-red-400 border border-red-500/20'
              }`}
            >
              {testResult.status === 'connected' ? <Check size={14} /> : <X size={14} />}
              {testResult.message}
            </div>
          )}

          {/* Save notification */}
          {saveMsg && (
            <div
              className={`text-xs px-3 py-2 rounded-lg mb-4 ${
                saveMsg === t('settings.saved')
                  ? 'bg-data-green/10 text-data-green border border-data-green/20'
                  : 'bg-red-500/10 text-red-400 border border-red-500/20'
              }`}
            >
              {saveMsg}
            </div>
          )}

          <div className="flex items-center gap-3">
            <button
              onClick={handleTest}
              disabled={(!config.api_key && !apiKeySet) || testing}
              className="flex items-center gap-1.5 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 text-gray-300 disabled:text-gray-600 text-xs px-4 py-2 rounded-lg transition-premium active:translate-y-[1px] disabled:cursor-not-allowed"
            >
              <Lightning size={14} />
              {testing ? t('settings.testing') : t('settings.testConnection')}
            </button>
            <button
              onClick={handleSave}
              disabled={saving}
              className="flex items-center gap-1.5 bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold text-xs px-4 py-2 rounded-lg transition-premium active:translate-y-[1px]"
            >
              <FloppyDisk size={14} weight="fill" />
              {saving ? t('settings.saving') : t('settings.saveConfiguration')}
            </button>
          </div>
        </div>
        )}

        {admin && <UsersPanel />}
      </div>
    </div>
  );
}

// ── Admin: user management ──
function UsersPanel() {
  const { t } = useTranslation();
  const me = getCurrentUser();
  const [users, setUsers] = useState<User[]>([]);
  const [loading, setLoading] = useState(true);
  const [msg, setMsg] = useState<{ type: 'ok' | 'err'; text: string } | null>(null);

  // New-user form
  const [showNew, setShowNew] = useState(false);
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [role, setRole] = useState('member');
  const [busy, setBusy] = useState(false);

  const flash = (type: 'ok' | 'err', text: string) => {
    setMsg({ type, text });
    setTimeout(() => setMsg(null), 4000);
  };

  const load = () => {
    setLoading(true);
    usersApi.list()
      .then(setUsers)
      .catch((e) => flash('err', e instanceof Error ? e.message : 'Failed to load users'))
      .finally(() => setLoading(false));
  };

  useEffect(() => { load(); }, []);

  const handleCreate = async () => {
    if (!username.trim() || password.length < 8) {
      flash('err', t('settings.users.validation'));
      return;
    }
    setBusy(true);
    try {
      await usersApi.create({ username: username.trim(), password, display_name: displayName.trim() || undefined, role });
      setUsername(''); setPassword(''); setDisplayName(''); setRole('member');
      setShowNew(false);
      flash('ok', t('settings.users.created'));
      load();
    } catch (e) {
      flash('err', e instanceof Error ? e.message : 'Failed to create user');
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async (u: User) => {
    if (!confirm(t('settings.users.deleteConfirm', { name: u.username }))) return;
    try {
      await usersApi.delete(u.id);
      flash('ok', t('settings.users.deleted'));
      load();
    } catch (e) {
      flash('err', e instanceof Error ? e.message : 'Failed to delete user');
    }
  };

  const handleToggleRole = async (u: User) => {
    const next = u.role === 'admin' ? 'member' : 'admin';
    try {
      await usersApi.update(u.id, { role: next });
      load();
    } catch (e) {
      flash('err', e instanceof Error ? e.message : 'Failed to update role');
    }
  };

  const inputCls = 'w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium';

  return (
    <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-sm font-semibold text-gray-200 flex items-center gap-2">
          <Users size={15} className="text-amber-500" />
          {t('settings.users.title')}
        </h2>
        <div className="flex items-center gap-2">
          {msg && (
            <span className={`text-[11px] px-2 py-1 rounded ${msg.type === 'ok' ? 'bg-green-500/10 text-green-400' : 'bg-red-500/10 text-red-400'}`}>
              {msg.text}
            </span>
          )}
          <button
            onClick={() => setShowNew((v) => !v)}
            className="flex items-center gap-1 bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold text-[11px] px-2.5 py-1.5 rounded-md transition-premium"
          >
            <Plus size={12} weight="bold" />
            {t('settings.users.add')}
          </button>
        </div>
      </div>

      {showNew && (
        <div className="grid grid-cols-2 gap-2 mb-4 p-3 bg-obsidian-950/50 rounded-lg border border-obsidian-700">
          <input className={inputCls} placeholder={t('settings.users.username')} value={username} onChange={(e) => setUsername(e.target.value)} />
          <input className={inputCls} placeholder={t('settings.users.displayName')} value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
          <input className={inputCls} type="password" placeholder={t('settings.users.password')} value={password} onChange={(e) => setPassword(e.target.value)} />
          <Select value={role} onChange={(v) => setRole(v)}>
            <option value="member">{t('settings.users.roleMember')}</option>
            <option value="admin">{t('settings.users.roleAdmin')}</option>
          </Select>
          <div className="col-span-2 flex justify-end gap-2">
            <button onClick={() => setShowNew(false)} className="text-[11px] text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700">
              {t('common.cancel')}
            </button>
            <button onClick={handleCreate} disabled={busy} className="text-[11px] text-[#08080c] bg-amber-500 hover:bg-amber-400 font-semibold px-3 py-1.5 rounded-md disabled:opacity-50">
              {busy ? t('common.loading') : t('common.save')}
            </button>
          </div>
        </div>
      )}

      {loading ? (
        <div className="text-xs text-gray-500 py-4 text-center">{t('common.loading')}</div>
      ) : (
        <div className="space-y-1">
          {users.map((u) => (
            <div key={u.id} className="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-obsidian-800/50 transition-premium">
              <div className="min-w-0 flex-1">
                <div className="text-xs text-gray-200 font-medium truncate">
                  {u.display_name || u.username}
                  {me?.id === u.id && <span className="text-[9px] text-gray-500 ml-1.5">({t('settings.users.you')})</span>}
                </div>
                <div className="text-[10px] text-gray-500">@{u.username}</div>
              </div>
              <button
                onClick={() => handleToggleRole(u)}
                disabled={me?.id === u.id}
                title={t('settings.users.toggleRole')}
                className={`text-[9px] px-2 py-0.5 rounded-full font-medium transition-premium disabled:opacity-50 disabled:cursor-not-allowed ${
                  u.role === 'admin' ? 'bg-amber-500/15 text-amber-500 hover:bg-amber-500/25' : 'bg-obsidian-700 text-gray-400 hover:bg-obsidian-600'
                }`}
              >
                {u.role === 'admin' ? t('settings.users.roleAdmin') : t('settings.users.roleMember')}
              </button>
              <button
                onClick={() => handleDelete(u)}
                disabled={me?.id === u.id}
                className="text-gray-600 hover:text-red-400 disabled:opacity-30 disabled:cursor-not-allowed transition-premium"
                title={t('common.delete')}
              >
                <Trash size={13} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}


