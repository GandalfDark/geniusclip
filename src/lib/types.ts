export type Codec = 'h264' | 'hevc' | 'av1';
export type Quality = 'low' | 'medium' | 'high' | 'ultra';
export type Resolution = 'native' | 'p720' | 'p1080' | 'p1440' | 'p2160';

export interface EngineConfig {
  monitor: string | null;
  fps: number;
  resolution: Resolution;
  codec: Codec;
  quality: Quality;
  bitrateKbps: number | null;
  captureCursor: boolean;
  /** Encode every frame even when the screen doesn't change (for video editors). */
  constantFps: boolean;
  systemAudio: boolean;
  systemDevice: string | null;
  systemVolume: number;
  mic: boolean;
  micDevice: string | null;
  micVolume: number;
  separateTracks: boolean;
  /** Discord's audio on a track of its own (with separate tracks). */
  voiceSeparate: boolean;
  diskBuffer: boolean;
  noiseSuppression: boolean;
  noiseStrength: number;
  micMuted: boolean;
}

/** Hardware load shown in the in-game menu. */
export interface SystemStats {
  cpu: number;
  ramUsedGb: number;
  ramTotalGb: number;
  gpu: number | null;
  gpuTemp: number | null;
}

export interface Hotkeys {
  saveClip: string;
  toggleReplay: string;
  screenshot: string;
  toggleRecording: string;
  saveShort: string;
  toggleMenu: string;
  togglePerf: string;
}

export interface Settings {
  engine: EngineConfig;
  replayEnabled: boolean;
  replaySeconds: number;
  skipSaved: boolean;
  shortSeconds: number;
  clipsDir: string;
  screenshotsDir: string;
  sortByGame: boolean;
  hotkeys: Hotkeys;
  autostart: boolean;
  /** 'auto' or a code from LANGS in i18n.ts. */
  language: string;
  accent: string;
  overlay: { enabled: boolean; corner: string; sound: boolean };
  perfOverlay: PerfOverlay;
  autoUpdate: boolean;
  pauseOnBattery: boolean;
  gamesOnly: boolean;
  autoClips: AutoClips;
  /** First-steps card on Home; the backend ticks the steps as they happen. */
  onboarding: Onboarding;
}

export interface Onboarding {
  clipSaved: boolean;
  menuOpened: boolean;
  dismissed: boolean;
}

export interface EngineStatus {
  running: boolean;
  replayEnabled: boolean;
  replaySeconds: number;
  recording: boolean;
  recordingSeconds: number;
  bufferSeconds: number;
  bufferBytes: number;
  encoder: string;
  width: number;
  height: number;
  fps: number;
  droppedFrames: number;
  droppedRecent: number;
  paused: boolean;
  lastError: string | null;
  noiseUnavailable: boolean;
  /** The codec asked for, when the GPU can't encode it and capture runs in H.264. */
  codecFallback: string | null;
}

export interface MonitorInfo {
  id: string;
  name: string;
  width: number;
  height: number;
  x: number;
  y: number;
  primary: boolean;
  hdr: boolean;
  adapter: string;
  vendorId: number;
}

export interface AudioDevice {
  id: string;
  name: string;
  isDefault: boolean;
  bluetooth: boolean;
}

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
}

export interface Snapshot {
  settings: Settings;
  status: EngineStatus;
  monitors: MonitorInfo[];
  audioOutputs: AudioDevice[];
  audioInputs: AudioDevice[];
  version: string;
  hotkeyErrors: string[];
  update: UpdateInfo | null;
  /** The system UI language mapped to a supported one. */
  lang: string;
  ramTotalMb: number;
  hasBattery: boolean;
  /** Set on the first start after an update, until dismissed. */
  whatsNew: WhatsNew | null;
  waitingForGame: boolean;
  supportedCodecs: string[] | null;
}

export interface WhatsNew {
  version: string;
  /** Release notes: lines starting with "- " are bullet points. */
  notes: string;
}

/** Free space on the clips folder's drive. */
export interface DiskSpace {
  /** "C:" */
  drive: string;
  freeMb: number;
  totalMb: number;
  /** Below this much free space `low` is set. */
  lowMb: number;
  low: boolean;
}

export type MediaKind = 'clip' | 'recording' | 'screenshot';

export interface MediaEntry {
  path: string;
  name: string;
  game: string;
  kind: MediaKind;
  size: number;
  modified: number;
  duration: number;
  width: number;
  height: number;
  favorite: boolean;
}

export interface Estimate {
  width: number;
  height: number;
  bitrateKbps: number;
  bufferMb: number;
}

export type EngineEvent =
  | { type: 'clipSaved'; path: string; seconds: number }
  | { type: 'clipFailed'; error: string }
  | { type: 'recordingStarted'; path: string }
  | { type: 'recordingSaved'; path: string }
  | { type: 'recordingFailed'; error: string }
  | { type: 'screenshotSaved'; path: string }
  | { type: 'screenshotFailed'; error: string }
  | { type: 'error'; message: string }
  | ({ type: 'status' } & EngineStatus);

/** Clips saved by game events (CS2 and Dota 2 Game State Integration). */
export interface AutoClips {
  cs2: boolean;
  cs2Events: string[];
  dota: boolean;
  dotaEvents: string[];
  token: string;
}

export interface AutoGameStatus {
  found: boolean;
  installed: boolean;
  connected: boolean;
  failed: boolean;
}

export interface AutoStatus {
  cs2: AutoGameStatus;
  dota: AutoGameStatus;
}

/** The in-game stats overlay. */
export interface PerfOverlay {
  enabled: boolean;
  style: 'line' | 'panel' | 'fps';
  corner: string;
  fps: boolean;
  gpu: boolean;
  cpu: boolean;
  clock: boolean;
}

/** Whether Windows lets this user read games' frame rates. */
export type PerfAccess = 'granted' | 'signInAgain' | 'needed';
