import { listen } from '@tauri-apps/api/event';
import { api } from './api';
import { applyAccent } from './accents';
import { isLang, matchLang, translate, type Lang, type TKey } from './i18n';
import type { EngineEvent, EngineStatus, MediaEntry, Settings, Snapshot, UpdateInfo } from './types';

export interface Notice {
  id: number;
  text: string;
  tone: 'ok' | 'error' | 'info';
}

class AppStore {
  snapshot = $state<Snapshot | null>(null);
  status = $state<EngineStatus | null>(null);
  media = $state<MediaEntry[]>([]);
  mediaLoaded = $state(false);
  update = $state<UpdateInfo | null>(null);
  hotkeyErrors = $state<string[]>([]);
  notices = $state<Notice[]>([]);
  savedPulse = $state(0);
  /** Bumped when a clip finishes saving (drives the button's "Saved" state). */
  clipSavedAt = $state(0);
  /** Paths saved during this session, highlighted briefly in lists. */
  fresh = $state<Record<string, true>>({});

  #systemLang: Lang = 'en';
  #saveTimer: ReturnType<typeof setTimeout> | null = null;
  #noticeId = 0;

  get settings(): Settings | null {
    return this.snapshot?.settings ?? null;
  }

  get lang(): Lang {
    const l = this.snapshot?.settings.language;
    return isLang(l) ? l : this.#systemLang;
  }

  t = (key: TKey, vars?: Record<string, string | number>) => translate(this.lang, key, vars);

  async init() {
    const snap = await api.snapshot();
    this.#systemLang = isLang(snap.lang) ? snap.lang : matchLang(navigator.language);
    this.snapshot = snap;
    this.status = snap.status;
    this.update = snap.update;
    this.hotkeyErrors = snap.hotkeyErrors;
    applyAccent(snap.settings.accent);
    document.documentElement.lang = this.lang;

    await listen<EngineStatus>('engine://status', (e) => (this.status = e.payload));
    await listen<Settings>('settings://changed', (e) => {
      if (this.snapshot) this.snapshot.settings = e.payload;
      applyAccent(e.payload.accent);
      api.hotkeyErrors().then((h) => (this.hotkeyErrors = h));
    });
    await listen<{ path: string } | null>('library://changed', (e) => {
      const path = e.payload?.path;
      if (path) {
        this.fresh[path] = true;
        setTimeout(() => delete this.fresh[path], 2200);
      }
      this.refreshMedia();
    });
    await listen<UpdateInfo>('update://available', (e) => (this.update = e.payload));
    await listen<EngineEvent>('engine://event', (e) => {
      const ev = e.payload;
      if (ev.type === 'clipSaved') this.clipSavedAt = Date.now();
      if (ev.type === 'clipFailed' || ev.type === 'recordingFailed' || ev.type === 'screenshotFailed') {
        this.notify(ev.error, 'error');
      }
    });
    this.refreshMedia();
  }

  async refreshMedia() {
    this.media = await api.listMedia();
    this.mediaLoaded = true;
  }

  async refreshSnapshot() {
    const snap = await api.snapshot();
    this.snapshot = snap;
    this.hotkeyErrors = snap.hotkeyErrors;
  }

  /** Applies a change locally right away and persists it (debounced). */
  change(mutate: (s: Settings) => void, delay = 250) {
    if (!this.snapshot) return;
    mutate(this.snapshot.settings);
    applyAccent(this.snapshot.settings.accent);
    if (this.#saveTimer) clearTimeout(this.#saveTimer);
    this.#saveTimer = setTimeout(() => this.#persist(), delay);
  }

  async #persist() {
    if (!this.snapshot) return;
    try {
      const saved = await api.updateSettings($state.snapshot(this.snapshot.settings));
      this.snapshot.settings = saved;
      this.hotkeyErrors = await api.hotkeyErrors();
      this.savedPulse++;
    } catch (e) {
      this.notify(String(e), 'error');
      await this.refreshSnapshot();
    }
  }

  notify(text: string, tone: Notice['tone'] = 'info') {
    const id = ++this.#noticeId;
    this.notices.push({ id, text, tone });
    setTimeout(() => (this.notices = this.notices.filter((n) => n.id !== id)), 4500);
  }
}

export const app = new AppStore();
