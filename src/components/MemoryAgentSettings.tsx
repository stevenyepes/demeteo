import React, { useState, useEffect } from 'react';
import { Brain, RotateCw, Check, Zap, RefreshCw } from 'lucide-react';
import {
  getMemoryAgentConfig,
  setMemoryAgentConfig,
  testMemoryAgentConnection,
  listMemoryAgentModels,
} from '../lib/project';
import type { MemoryAgentConfig, MemoryAgentTestResult } from '../types';

const EMPTY: MemoryAgentConfig = {
  enabled: false,
  chat_endpoint: 'http://localhost:11434/v1',
  chat_model: '',
  embed_endpoint: 'http://localhost:11434/v1',
  embed_model: '',
  has_api_key: false,
  top_k: 12,
  min_confidence: 0,
};

const MemoryAgentSettings: React.FC = () => {
  const [config, setConfig] = useState<MemoryAgentConfig>(EMPTY);
  const [apiKey, setApiKey] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<MemoryAgentTestResult | null>(null);
  const [chatModels, setChatModels] = useState<string[]>([]);
  const [embedModels, setEmbedModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);

  const loadModels = async (cfg: MemoryAgentConfig, key?: string) => {
    if (!cfg.chat_endpoint && !cfg.embed_endpoint) return;
    setModelsLoading(true);
    setModelsError(null);
    try {
      const chat = cfg.chat_endpoint
        ? await listMemoryAgentModels(cfg.chat_endpoint, key)
        : [];
      const embed =
        cfg.embed_endpoint && cfg.embed_endpoint !== cfg.chat_endpoint
          ? await listMemoryAgentModels(cfg.embed_endpoint, key)
          : chat;
      setChatModels(chat);
      setEmbedModels(embed);
      if (chat.length === 0 && embed.length === 0) {
        setModelsError('no models returned by endpoint');
      }
    } catch (e) {
      setModelsError(String(e));
    } finally {
      setModelsLoading(false);
    }
  };

  useEffect(() => {
    getMemoryAgentConfig()
      .then((c) => {
        const merged = { ...EMPTY, ...c };
        setConfig(merged);
        void loadModels(merged);
      })
      .catch((e) => console.error('Failed to load memory agent config:', e))
      .finally(() => setLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const update = <K extends keyof MemoryAgentConfig>(key: K, value: MemoryAgentConfig[K]) => {
    setConfig((c) => ({ ...c, [key]: value }));
    setSaved(false);
  };

  const handleSave = async () => {
    setSaving(true);
    setSaved(false);
    try {
      // `apiKey` left blank means "keep stored key"; send undefined.
      await setMemoryAgentConfig(config, apiKey === '' ? undefined : apiKey);
      setApiKey('');
      const fresh = await getMemoryAgentConfig();
      setConfig({ ...EMPTY, ...fresh });
      setSaved(true);
    } catch (e) {
      console.error('Failed to save memory agent config:', e);
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const res = await testMemoryAgentConnection(config, apiKey === '' ? undefined : apiKey);
      setTestResult(res);
    } catch (e) {
      setTestResult({ chat_ok: false, embed_ok: false, embed_dims: null, error: String(e) });
    } finally {
      setTesting(false);
    }
  };

  if (loading) {
    return (
      <div className="glass-panel p-6 flex items-center justify-center text-slate-400 text-xs">
        <RotateCw className="w-4 h-4 animate-spin mr-2" /> Loading…
      </div>
    );
  }

  const field = (label: string, hint?: string) => (
    <label className="block text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1.5">
      {label}
      {hint && <span className="normal-case text-slate-600"> {hint}</span>}
    </label>
  );
  const inputCls =
    'w-full bg-black/40 border border-white/10 rounded-lg px-3 py-2 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600';

  return (
    <div className="space-y-4">
      <div className="glass-panel p-6">
        <div className="flex items-start justify-between mb-1">
          <h3 className="text-sm font-outfit font-semibold text-white flex items-center gap-2">
            <Brain className="w-4 h-4 text-violet-400" />
            Memory Agent
          </h3>
          <label className="flex items-center gap-2 text-xs text-slate-300 cursor-pointer">
            <input
              type="checkbox"
              checked={config.enabled}
              onChange={(e) => update('enabled', e.target.checked)}
              className="accent-violet-500"
            />
            Enabled
          </label>
        </div>
        <p className="text-xs text-slate-400 mb-4">
          When enabled, Demeteo distills signals from your runs (gate feedback, failures, agent
          summaries) into project memories using a local or OpenAI-compatible LLM, and injects the
          most relevant ones into future agent prompts. Runs in the background against the endpoint
          you configure below (e.g. Ollama).
        </p>

        <div className="grid grid-cols-2 gap-3">
          <div>
            {field('Chat endpoint')}
            <input
              className={inputCls}
              value={config.chat_endpoint}
              placeholder="http://localhost:11434/v1"
              onChange={(e) => update('chat_endpoint', e.target.value)}
            />
          </div>
          <div>
            <div className="flex items-center justify-between">
              {field('Chat model')}
              <button
                type="button"
                onClick={() => loadModels(config, apiKey === '' ? undefined : apiKey)}
                disabled={modelsLoading}
                className="mb-1.5 text-[10px] font-mono text-slate-500 hover:text-cyan-400 flex items-center gap-1 disabled:opacity-50"
                title="Fetch models from the endpoint"
              >
                <RefreshCw className={`w-3 h-3 ${modelsLoading ? 'animate-spin' : ''}`} />
                {modelsLoading ? 'loading' : 'refresh'}
              </button>
            </div>
            <input
              className={inputCls}
              list="mem-chat-models"
              value={config.chat_model}
              placeholder={chatModels[0] ?? 'llama3.1'}
              onChange={(e) => update('chat_model', e.target.value)}
            />
            <datalist id="mem-chat-models">
              {chatModels.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          </div>
          <div>
            {field('Embeddings endpoint', '(blank = use chat)')}
            <input
              className={inputCls}
              value={config.embed_endpoint}
              placeholder={config.chat_endpoint || 'http://localhost:11434/v1'}
              onChange={(e) => update('embed_endpoint', e.target.value)}
            />
          </div>
          <div>
            {field('Embeddings model')}
            <input
              className={inputCls}
              list="mem-embed-models"
              value={config.embed_model}
              placeholder="nomic-embed-text"
              onChange={(e) => update('embed_model', e.target.value)}
            />
            <datalist id="mem-embed-models">
              {embedModels.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
            {modelsError && (
              <p className="mt-1 text-[10px] font-mono text-amber-500/80">
                models: {modelsError} — type the model name manually
              </p>
            )}
          </div>
          <div>
            {field('API key', config.has_api_key ? '(stored — blank keeps it)' : '(optional)')}
            <input
              type="password"
              className={inputCls}
              value={apiKey}
              placeholder={config.has_api_key ? '••••••••' : 'leave blank for local models'}
              onChange={(e) => setApiKey(e.target.value)}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              {field('Top K')}
              <input
                type="number"
                min={1}
                className={inputCls}
                value={config.top_k}
                onChange={(e) => update('top_k', Math.max(1, Number(e.target.value) || 1))}
              />
            </div>
            <div>
              {field('Min conf.')}
              <input
                type="number"
                min={0}
                max={1}
                step={0.05}
                className={inputCls}
                value={config.min_confidence}
                onChange={(e) =>
                  update('min_confidence', Math.min(1, Math.max(0, Number(e.target.value) || 0)))
                }
              />
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2 mt-4">
          <button
            onClick={handleTest}
            disabled={testing}
            className="px-3 py-2 text-xs font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 disabled:opacity-50 text-slate-300 transition-all flex items-center gap-1.5"
          >
            {testing ? <RotateCw className="w-3.5 h-3.5 animate-spin" /> : <Zap className="w-3.5 h-3.5" />}
            Test connection
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="px-4 py-2 text-xs font-medium rounded-lg bg-cyan-600 hover:bg-cyan-500 disabled:opacity-50 text-white transition-all flex items-center gap-1.5"
          >
            {saving ? <RotateCw className="w-3 h-3 animate-spin" /> : saved ? <Check className="w-3 h-3" /> : null}
            {saved ? 'Saved' : 'Save'}
          </button>

          {testResult && (
            <span
              className={`text-[11px] font-mono ${
                testResult.chat_ok && testResult.embed_ok ? 'text-emerald-400' : 'text-ruby-400'
              }`}
            >
              {testResult.chat_ok && testResult.embed_ok
                ? `OK — chat + embeddings (${testResult.embed_dims ?? '?'} dims)`
                : testResult.error ?? 'connection failed'}
            </span>
          )}
        </div>
      </div>
    </div>
  );
};

export default MemoryAgentSettings;
