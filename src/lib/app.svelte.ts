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
  /** Set by the hidden in-game menu: library changes are skipped until it
   *  refreshes on its next open. */
  mediaPaused = false;

  #systemLang: Lang = 'en';
  #saveTimer: ReturnType<typeof setTimeout> | null = null;
  /** Settings saves in flight. */
  #saving = 0;
  #persistSeq = 0;
  /** Newest settings from the backend that arrived while a local change was
   *  pending; applied once it is saved, so edits made meanwhile survive. */
  #remote: Settings | null = null;
  #mediaSeq = 0;
  #mediaReq: Promise<void> | null = null;
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
      this.#remote = e.payload;
      if (!this.#pending) this.#applyRemote();
      api.hotkeyErrors().then((h) => (this.hotkeyErrors = h));
    });
    await listen<{ path: string } | null>('library://changed', (e) => {
      if (this.mediaPaused) return;
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

  /** Resolves once the newest listing is in: responses can arrive out of
   *  order, and an older one must not overwrite a newer one. */
  refreshMedia(): Promise<void> {
    const seq = ++this.#mediaSeq;
    const req = api.listMedia().then((list) => {
      // A newer request was made meanwhile: wait for that one instead.
      if (seq !== this.#mediaSeq) return this.#mediaReq!;
      this.media = list;
      this.mediaLoaded = true;
    });
    this.#mediaReq = req;
    return req;
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
    this.#saveTimer = setTimeout(() => {
      this.#saveTimer = null;
      this.#persist();
    }, delay);
  }

  /** A local change is waiting to be saved or being saved. */
  get #pending() {
    return this.#saveTimer !== null || this.#saving > 0;
  }

  #applyRemote() {
    const s = this.#remote;
    this.#remote = null;
    if (!s || !this.snapshot) return;
    this.snapshot.settings = s;
    applyAccent(s.accent);
  }

  async #persist() {
    if (!this.snapshot) return;
    const seq = ++this.#persistSeq;
    this.#saving++;
    let failed = false;
    try {
      // Whatever arrived before this save is replaced by it.
      this.#remote = null;
      const saved = await api.updateSettings($state.snapshot(this.snapshot.settings));
      if (seq === this.#persistSeq) this.#remote = saved;
      this.hotkeyErrors = await api.hotkeyErrors();
      this.savedPulse++;
    } catch (e) {
      this.notify(String(e), 'error');
      failed = true;
    } finally {
      this.#saving--;
    }
    // A newer edit is on its way and will bring its own result.
    if (this.#pending) return;
    if (failed) {
      this.#remote = null;
      await this.refreshSnapshot();
    } else {
      this.#applyRemote();
    }
  }

  notify(text: string, tone: Notice['tone'] = 'info') {
    const id = ++this.#noticeId;
    this.notices.push({ id, text, tone });
    setTimeout(() => (this.notices = this.notices.filter((n) => n.id !== id)), 4500);
  }
}

export const app = new AppStore();
