import { listen } from '@tauri-apps/api/event';
import { api } from './api';
import { applyAccent } from './accents';
import { isKey, isLang, matchLang, translate, type Lang, type TKey } from './i18n';
import type { EngineEvent, EngineStatus, MediaEntry, Settings, Snapshot, UpdateInfo } from './types';

export interface Notice {
  id: number;
  text: string;
  tone: 'ok' | 'error' | 'info';
}

/** library://changed: the file's new listing, or that it's gone. Neither
 *  (or no payload) means "something changed, list again". */
type LibraryChange = { path: string; entry?: MediaEntry | null; removed?: boolean };

/** `list` without `path` (and `entry`'s own path), plus `entry` placed like
 *  the backend sorts: newest first. */
function upsert(list: MediaEntry[], path: string, entry: MediaEntry | null): MediaEntry[] {
  const out = list.filter((m) => m.path !== path && m.path !== entry?.path);
  if (entry) {
    const i = out.findIndex((m) => m.modified < entry.modified);
    out.splice(i < 0 ? out.length : i, 0, entry);
  }
  return out;
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
  /** Listings requested and not answered yet. */
  #listing = 0;
  #noticeId = 0;
  /** Favorite requests made per path, so an older answer can't win. */
  #favSeq: Record<string, number> = {};

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

    await listen<EngineStatus>('engine://status', (e) => (this.status = e.payload));
    await listen<Settings>('settings://changed', (e) => {
      this.#remote = e.payload;
      if (!this.#pending) this.#applyRemote();
      api.hotkeyErrors().then((h) => (this.hotkeyErrors = h));
    });
    await listen<LibraryChange | null>('library://changed', (e) => {
      if (this.mediaPaused) return;
      const c = e.payload;
      if (c?.removed) return this.#patch(c.path, null);
      const path = c?.entry?.path ?? c?.path;
      // A new or rewritten file is highlighted; one only starred or unstarred is not.
      const next = c?.entry;
      const same = !!next && this.media.some((m) => m.path === next.path && m.modified === next.modified && m.size === next.size);
      if (path && !same) {
        this.fresh[path] = true;
        setTimeout(() => delete this.fresh[path], 2200);
      }
      if (c?.entry) this.#patch(c.path, c.entry);
      else this.refreshMedia();
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
    this.#listing++;
    const req = api
      .listMedia()
      .finally(() => this.#listing--)
      .then((list) => {
        // A newer request was made meanwhile: wait for that one instead.
        if (seq !== this.#mediaSeq) return this.#mediaReq!;
        this.media = list;
        this.mediaLoaded = true;
      });
    this.#mediaReq = req;
    return req;
  }

  /** One file changed: updated in place, without listing the whole library. */
  #patch(path: string, entry: MediaEntry | null) {
    this.media = upsert(this.media, path, entry);
    // A listing under way may have been taken before this change and would
    // undo it: a newer one replaces it.
    if (this.#listing) this.refreshMedia();
  }

  /** Stars a file or takes the star off: shown at once, then replaced by the
   *  backend's listing (only the newest request's, on quick repeated clicks). */
  async setFavorite(entry: MediaEntry, on: boolean) {
    const path = entry.path;
    const seq = (this.#favSeq[path] = (this.#favSeq[path] ?? 0) + 1);
    const current = this.media.find((m) => m.path === path);
    if (current) this.#patch(path, { ...current, favorite: on });
    try {
      const updated = await api.setFavorite(path, on);
      if (this.#favSeq[path] === seq) this.#patch(path, updated);
    } catch (e) {
      if (this.#favSeq[path] === seq) {
        const now = this.media.find((m) => m.path === path);
        if (now) this.#patch(path, { ...now, favorite: !on });
      }
      this.notify(String(e), 'error');
    }
  }

  /** Hides the "what's new" card for good. */
  async dismissWhatsNew() {
    if (!this.snapshot) return;
    this.snapshot.whatsNew = null;
    try {
      await api.dismissWhatsNew();
    } catch (e) {
      this.notify(String(e), 'error');
    }
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

  /** Backend errors are stable codes ("err.not-found", maybe followed by
   *  details), shown translated; a code this version doesn't know becomes
   *  err.unknown. Any other text is shown as is. */
  errorText(msg: string): string {
    const code = /^(?:Error:\s*)?(err\.[\w-]+)/.exec(msg.trim())?.[1];
    if (!code) return msg;
    return this.t(isKey(code) ? code : 'err.unknown');
  }

  notify(msg: string, tone: Notice['tone'] = 'info') {
    const text = this.errorText(msg);
    // The code (and any details) stays findable in the console.
    if (text !== msg) console.warn('error:', msg);
    const id = ++this.#noticeId;
    this.notices.push({ id, text, tone });
    setTimeout(() => (this.notices = this.notices.filter((n) => n.id !== id)), 4500);
  }
}

export const app = new AppStore();
