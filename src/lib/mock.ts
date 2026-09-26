// Browser-only preview: fakes the Tauri backend so the UI can be designed
// and reviewed in a normal browser (`npm run dev`, open localhost:1420).
// Never loaded inside the real app.
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import type { MediaEntry, Settings } from './types';

import { emit } from '@tauri-apps/api/event';

const palette = [
  ['#ff6b9d', '#6d28d9'],
  ['#22d3ee', '#1e3a8a'],
  ['#fbbf24', '#b91c1c'],
  ['#34d399', '#155e75'],
  ['#a78bfa', '#312e81'],
  ['#fb7185', '#7c2d12'],
];

function thumb(i: number, label: string) {
  const [a, b] = palette[i % palette.length];
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='480' height='270'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='${a}'/><stop offset='1' stop-color='${b}'/></linearGradient></defs><rect width='480' height='270' fill='url(#g)'/><circle cx='${120 + i * 40}' cy='${90 + (i % 3) * 30}' r='70' fill='white' fill-opacity='0.12'/><text x='24' y='246' font-family='Segoe UI' font-size='28' font-weight='700' fill='white' fill-opacity='0.85'>${label}</text></svg>`;
  return 'data:image/svg+xml,' + encodeURIComponent(svg);
}

const games = ['Counter-Strike 2', 'Brawlhalla', 'VALORANT', 'Dota 2', 'Minecraft', 'Apex Legends'];
const now = Date.now();
const media: MediaEntry[] = Array.from({ length: 14 }, (_, i) => {
  const kind = i % 5 === 3 ? 'screenshot' : i % 7 === 5 ? 'recording' : 'clip';
  const game = games[i % games.length];
  return {
    path: `mock://${i}`,
    name: `${game} 2026-09-${String(26 - Math.floor(i / 3)).padStart(2, '0')} 21-${String(10 + i).padStart(2, '0')}-00`,
    game,
    kind,
    size: kind === 'screenshot' ? 3_800_000 : 90_000_000 + i * 17_000_000,
    modified: now - i * 3.7 * 3600_000,
    duration: kind === 'screenshot' ? 0 : kind === 'recording' ? 1260 + i * 30 : [300, 42, 75, 300, 118][i % 5],
    width: 2560,
    height: 1440,
  };
});

// A real local video (static/dev-clip.mp4, not in git) to test the player layout.
media.unshift({
  path: '/dev-clip.mp4',
  name: 'Dev clip',
  game: 'Brawlhalla',
  kind: 'clip',
  size: 1_196_567,
  modified: now,
  duration: 3,
  width: 2560,
  height: 1440,
});

let settings: Settings = {
  engine: {
    monitor: null,
    fps: 60,
    resolution: 'native',
    codec: 'h264',
    quality: 'high',
    bitrateKbps: null,
    captureCursor: true,
    systemAudio: true,
    systemDevice: null,
    systemVolume: 1,
    mic: true,
    micDevice: null,
    micVolume: 1,
    separateTracks: true,
  },
  replayEnabled: true,
  replaySeconds: 300,
  skipSaved: true,
  clipsDir: 'C:\\Users\\Player\\Videos\\GeniusClip',
  screenshotsDir: 'C:\\Users\\Player\\Pictures\\GeniusClip',
  sortByGame: true,
  hotkeys: { saveClip: 'Alt+F8', toggleReplay: 'Alt+Shift+F8', screenshot: 'Alt+F6', toggleRecording: 'Alt+F7' },
  autostart: true,
  language: 'auto',
  accent: 'violet',
  overlay: { enabled: true, corner: 'top-right', sound: true },
  autoUpdate: true,
};

let buffer = 247;
let recording = false;
let recStart = 0;

function status() {
  return {
    running: settings.replayEnabled,
    replayEnabled: settings.replayEnabled,
    replaySeconds: settings.replaySeconds,
    recording,
    recordingSeconds: recording ? (Date.now() - recStart) / 1000 : 0,
    bufferSeconds: Math.min(buffer, settings.replaySeconds),
    bufferBytes: Math.min(buffer, settings.replaySeconds) * 3_900_000,
    encoder: 'h264_nvenc',
    width: 2560,
    height: 1440,
    fps: 60,
    droppedFrames: 0,
    lastError: null,
  };
}

export function installMock() {
  (window as any).__GC_MOCK__ = true;
  mockWindows('main');
  mockIPC(
    (cmd, args: any) => {
      switch (cmd) {
        case 'get_snapshot':
          return {
            settings,
            status: status(),
            monitors: [
              { id: '\\\\.\\DISPLAY1', name: 'X27U', width: 2560, height: 1440, x: 0, y: 0, primary: true, hdr: false, adapter: 'NVIDIA GeForce RTX 4090', vendorId: 4318 },
              { id: '\\\\.\\DISPLAY2', name: 'VG240Y', width: 1920, height: 1080, x: 2560, y: 0, primary: false, hdr: false, adapter: 'NVIDIA GeForce RTX 4090', vendorId: 4318 },
            ],
            audioOutputs: [
              { id: 'o1', name: 'Динамики (Realtek Audio)', isDefault: true },
              { id: 'o2', name: 'Наушники (HyperX Cloud II)', isDefault: false },
            ],
            audioInputs: [{ id: 'i1', name: 'Микрофон (HyperX Cloud II)', isDefault: true }],
            version: '0.1.0',
            hotkeyErrors: [],
            update: null,
            lang: navigator.language.startsWith('ru') ? 'ru' : 'en',
          };
        case 'get_status':
          return status();
        case 'estimate': {
          const s = args.settings as Settings;
          const ref = { low: 8, medium: 14, high: 20, ultra: 32 }[s.engine.quality];
          const h = { native: 1440, p720: 720, p1080: 1080, p1440: 1440, p2160: 2160 }[s.engine.resolution];
          const w = Math.round((h * 16) / 9);
          const mbps = ref * Math.pow((w * h * s.engine.fps) / (1920 * 1080 * 60), 0.75) * { h264: 1, hevc: 0.7, av1: 0.65 }[s.engine.codec];
          return { width: w, height: h, bitrateKbps: Math.round(mbps * 1000), bufferMb: Math.round((mbps / 8) * s.replaySeconds) };
        }
        case 'update_settings':
          settings = args.settings;
          return settings;
        case 'set_replay_enabled':
          settings.replayEnabled = args.on;
          return;
        case 'save_clip': {
          const i = media.length;
          const entry: MediaEntry = {
            path: `mock://${i}`,
            name: `Brawlhalla new ${i}`,
            game: 'Brawlhalla',
            kind: 'clip',
            size: 48_000_000,
            modified: Date.now(),
            duration: 10 + (i % 50),
            width: 2560,
            height: 1440,
          };
          setTimeout(() => {
            media.unshift(entry);
            emit('engine://event', { type: 'clipSaved', path: entry.path, seconds: entry.duration });
            emit('library://changed', entry);
          }, 350);
          return;
        }
        case 'toggle_recording':
          recording = !recording;
          recStart = Date.now();
          return;
        case 'hotkey_errors':
          return [];
        case 'list_media':
          return media;
        case 'thumbnail': {
          const i = Number(String(args.path).split('//')[1]);
          return thumb(i, media.find((m) => m.path === args.path)?.game ?? '');
        }
        case 'plugin:event|listen':
          return Math.floor(Math.random() * 1e6);
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  setInterval(() => {
    buffer += 1;
    emit('engine://status', status());
  }, 1000);
}
