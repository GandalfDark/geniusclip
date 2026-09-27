import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import type { Estimate, MediaEntry, Settings, Snapshot, EngineStatus, UpdateInfo, SystemStats } from './types';

export const api = {
  snapshot: () => invoke<Snapshot>('get_snapshot'),
  status: () => invoke<EngineStatus>('get_status'),
  estimate: (settings: Settings) => invoke<Estimate>('estimate', { settings }),
  updateSettings: (settings: Settings) => invoke<Settings>('update_settings', { settings }),
  setReplay: (on: boolean) => invoke<void>('set_replay_enabled', { on }),
  saveClip: () => invoke<void>('save_clip'),
  screenshot: () => invoke<void>('take_screenshot'),
  toggleRecording: () => invoke<void>('toggle_recording'),
  hotkeyErrors: () => invoke<string[]>('hotkey_errors'),
  /** Global hotkeys off while a hotkey field waits for a combo, so pressing
   *  an existing one is recorded instead of firing. */
  /** Resolves to the hotkey conflicts (as `hotkeyErrors`) once registered again. */
  setHotkeysSuspended: (suspended: boolean) => invoke<string[]>('set_hotkeys_suspended', { suspended }),
  listMedia: () => invoke<MediaEntry[]>('list_media'),
  thumbnail: (path: string) => invoke<string>('thumbnail', { path }),
  deleteMedia: (path: string) => invoke<void>('delete_media', { path }),
  renameMedia: (path: string, name: string) => invoke<string>('rename_media', { path, name }),
  trimMedia: (path: string, start: number, end: number, replace: boolean, gains: number[] | null = null) =>
    invoke<MediaEntry | null>('trim_media', { path, start, end, replace, gains }),
  clipAudio: (path: string) => invoke<ClipAudio>('clip_audio', { path }),
  openPath: (path: string) => invoke<void>('open_path', { path }),
  revealPath: (path: string) => invoke<void>('reveal_path', { path }),
  openMediaDir: (screenshots: boolean) => invoke<void>('open_media_dir', { screenshots }),
  checkUpdate: () => invoke<UpdateInfo | null>('check_update'),
  installUpdate: () => invoke<void>('install_update'),
  copyMedia: (paths: string[]) => invoke<void>('copy_media', { paths }),
  micTest: (on: boolean) => invoke<void>('mic_test', { on }),
  menuClose: () => invoke<void>('menu_close'),
  menuScreenshot: () => invoke<void>('menu_screenshot'),
  menuReady: () => invoke<void>('menu_ready'),
  systemStats: () => invoke<SystemStats>('system_stats'),
  /** Main window on a page, optionally opening a clip in the viewer. */
  openInApp: (route: string, path: string | null = null) => invoke<void>('open_in_app', { route, path }),
  takePendingOpen: () => invoke<string | null>('take_pending_open'),
  closeMain: () => invoke<void>('close_main'),
  quit: () => invoke<void>('quit_app'),
};

/** Audio tracks of a clip, extracted for the trim preview. */
export interface ClipAudio {
  titles: string[];
  /** GeniusClip layout: track 0 mixes game (1) and mic (2). */
  mix: boolean;
  files: string[];
  /** Peak level 0..1 per slice of the clip; empty for the mix track. */
  peaks: number[][];
}

export const fileUrl = (path: string) =>
  path.startsWith('data:') || (window as { __GC_MOCK__?: boolean }).__GC_MOCK__ ? path : convertFileSrc(path);
