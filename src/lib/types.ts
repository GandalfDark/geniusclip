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
  systemAudio: boolean;
  systemDevice: string | null;
  systemVolume: number;
  mic: boolean;
  micDevice: string | null;
  micVolume: number;
  separateTracks: boolean;
  diskBuffer: boolean;
  noiseSuppression: boolean;
  noiseStrength: number;
}

export interface Hotkeys {
  saveClip: string;
  toggleReplay: string;
  screenshot: string;
  toggleRecording: string;
  saveShort: string;
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
  autoUpdate: boolean;
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
  lastError: string | null;
  noiseUnavailable: boolean;
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
  | { type: 'status' };
