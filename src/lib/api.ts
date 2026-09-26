import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import type { Estimate, MediaEntry, Settings, Snapshot, EngineStatus, UpdateInfo } from './types';

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
  listMedia: () => invoke<MediaEntry[]>('list_media'),
  thumbnail: (path: string) => invoke<string>('thumbnail', { path }),
  deleteMedia: (path: string) => invoke<void>('delete_media', { path }),
  renameMedia: (path: string, name: string) => invoke<string>('rename_media', { path, name }),
  trimMedia: (path: string, start: number, end: number, replace: boolean) =>
    invoke<MediaEntry | null>('trim_media', { path, start, end, replace }),
  openPath: (path: string) => invoke<void>('open_path', { path }),
  revealPath: (path: string) => invoke<void>('reveal_path', { path }),
  openMediaDir: (screenshots: boolean) => invoke<void>('open_media_dir', { screenshots }),
  checkUpdate: () => invoke<UpdateInfo | null>('check_update'),
  installUpdate: () => invoke<void>('install_update'),
  copyMedia: (paths: string[]) => invoke<void>('copy_media', { paths }),
  closeMain: () => invoke<void>('close_main'),
  quit: () => invoke<void>('quit_app'),
};

export const fileUrl = (path: string) =>
  path.startsWith('data:') || (window as { __GC_MOCK__?: boolean }).__GC_MOCK__ ? path : convertFileSrc(path);
