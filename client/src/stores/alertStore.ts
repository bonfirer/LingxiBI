import { create } from 'zustand';
import { isAdmin } from '../lib/currentUser';
import type {
  AlertRule,
  AlertLog,
  SmtpConfig,
  FeishuConfig,
  CreateAlertRulePayload,
  UpdateAlertRulePayload,
  UpdateSmtpConfigPayload,
  UpdateFeishuConfigPayload,
} from '../lib/types';
import { alertsApi } from '../lib/api';

interface AlertStore {
  rules: AlertRule[];
  logs: AlertLog[];
  smtp: SmtpConfig | null;
  feishu: FeishuConfig | null;
  loading: boolean;
  error: string | null;

  fetchRules: () => Promise<void>;
  createRule: (payload: CreateAlertRulePayload) => Promise<AlertRule>;
  updateRule: (id: number, payload: UpdateAlertRulePayload) => Promise<AlertRule>;
  deleteRule: (id: number) => Promise<void>;

  fetchSmtp: () => Promise<void>;
  updateSmtp: (payload: UpdateSmtpConfigPayload) => Promise<void>;

  fetchFeishu: () => Promise<void>;
  updateFeishu: (payload: UpdateFeishuConfigPayload) => Promise<void>;

  fetchLogs: (params?: { rule_id?: number; limit?: number }) => Promise<void>;
  setError: (error: string | null) => void;
}

export const useAlertStore = create<AlertStore>((set) => ({
  rules: [],
  logs: [],
  smtp: null,
  feishu: null,
  loading: false,
  error: null,

  fetchRules: async () => {
    set({ loading: true, error: null });
    try {
      const rules = await alertsApi.listRules();
      set({ rules, loading: false });
    } catch (e: unknown) {
      set({ error: (e as Error).message, loading: false });
    }
  },

  createRule: async (payload) => {
    set({ loading: true, error: null });
    try {
      const rule = await alertsApi.createRule(payload);
      set((s) => ({ rules: [rule, ...s.rules], loading: false }));
      return rule;
    } catch (e: unknown) {
      set({ error: (e as Error).message, loading: false });
      throw e;
    }
  },

  updateRule: async (id, payload) => {
    set({ loading: true, error: null });
    try {
      const rule = await alertsApi.updateRule(id, payload);
      set((s) => ({ rules: s.rules.map((r) => (r.id === id ? rule : r)), loading: false }));
      return rule;
    } catch (e: unknown) {
      set({ error: (e as Error).message, loading: false });
      throw e;
    }
  },

  deleteRule: async (id) => {
    set({ loading: true, error: null });
    try {
      await alertsApi.deleteRule(id);
      set((s) => ({ rules: s.rules.filter((r) => r.id !== id), loading: false }));
    } catch (e: unknown) {
      set({ error: (e as Error).message, loading: false });
      throw e;
    }
  },

  // SMTP/Feishu delivery settings are shared infrastructure and the API now
  // restricts reading them to admins. Skip the call for members rather than
  // surfacing a 403 they can't act on.
  fetchSmtp: async () => {
    if (!isAdmin()) return;
    try {
      const smtp = await alertsApi.getSmtp();
      set({ smtp });
    } catch (e: unknown) {
      set({ error: (e as Error).message });
    }
  },

  updateSmtp: async (payload) => {
    set({ loading: true, error: null });
    try {
      const smtp = await alertsApi.updateSmtp(payload);
      set({ smtp, loading: false });
    } catch (e: unknown) {
      set({ error: (e as Error).message, loading: false });
      throw e;
    }
  },

  fetchFeishu: async () => {
    if (!isAdmin()) return;
    try {
      const feishu = await alertsApi.getFeishu();
      set({ feishu });
    } catch (e: unknown) {
      set({ error: (e as Error).message });
    }
  },

  updateFeishu: async (payload) => {
    set({ loading: true, error: null });
    try {
      const feishu = await alertsApi.updateFeishu(payload);
      set({ feishu, loading: false });
    } catch (e: unknown) {
      set({ error: (e as Error).message, loading: false });
      throw e;
    }
  },

  fetchLogs: async (params) => {
    try {
      const logs = await alertsApi.listLogs(params);
      set({ logs });
    } catch (e: unknown) {
      set({ error: (e as Error).message });
    }
  },

  setError: (error) => set({ error }),
}));
