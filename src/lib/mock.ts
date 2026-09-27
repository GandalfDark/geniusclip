// Browser-only preview: fakes the Tauri backend so the UI can be designed
// and reviewed in a normal browser (`npm run dev`, open localhost:1420).
// Never loaded inside the real app.
//
// URL options: ?lang=en picks the UI language; ?clean hides the one-off
// cards (what's new, first steps, low disk) for website screenshots.
// In the console, __gcMock.openMenu() ticks the "open the menu" step.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { MediaEntry, Settings } from "./types";

import { emit } from "@tauri-apps/api/event";

const palette = [
  ["#ff6b9d", "#6d28d9"],
  ["#22d3ee", "#1e3a8a"],
  ["#fbbf24", "#b91c1c"],
  ["#34d399", "#155e75"],
  ["#a78bfa", "#312e81"],
  ["#fb7185", "#7c2d12"],
];

function thumb(i: number, label: string) {
  const [a, b] = palette[i % palette.length];
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='480' height='270'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='${a}'/><stop offset='1' stop-color='${b}'/></linearGradient></defs><rect width='480' height='270' fill='url(#g)'/><circle cx='${120 + i * 40}' cy='${90 + (i % 3) * 30}' r='70' fill='white' fill-opacity='0.12'/><text x='24' y='246' font-family='Segoe UI' font-size='28' font-weight='700' fill='white' fill-opacity='0.85'>${label}</text></svg>`;
  return "data:image/svg+xml," + encodeURIComponent(svg);
}

const games = [
  "Counter-Strike 2",
  "Brawlhalla",
  "VALORANT",
  "Dota 2",
  "Minecraft",
  "Apex Legends",
];
const now = Date.now();
const media: MediaEntry[] = Array.from({ length: 14 }, (_, i) => {
  const kind = i % 5 === 3 ? "screenshot" : i % 7 === 5 ? "recording" : "clip";
  const game = games[i % games.length];
  return {
    path: `mock://${i}`,
    name: `${game} 2026-09-${String(26 - Math.floor(i / 3)).padStart(2, "0")} 21-${String(10 + i).padStart(2, "0")}-00`,
    game,
    kind,
    size: kind === "screenshot" ? 3_800_000 : 90_000_000 + i * 17_000_000,
    modified: now - i * 3.7 * 3600_000,
    duration:
      kind === "screenshot"
        ? 0
        : kind === "recording"
          ? 1260 + i * 30
          : [300, 42, 75, 300, 118][i % 5],
    width: 2560,
    height: 1440,
    favorite: i === 2 || i === 6 || i === 9,
  };
});

// A real local video (static/dev-clip.mp4, not in git) to test the player layout.
media.unshift({
  path: "/dev-clip.mp4",
  name: "Brawlhalla 2026-09-26 21-58-04",
  game: "Brawlhalla",
  kind: "clip",
  size: 18_547_707,
  modified: now,
  duration: 24,
  width: 2560,
  height: 1440,
  favorite: true,
});

let settings: Settings = {
  engine: {
    monitor: null,
    fps: 60,
    resolution: "native",
    codec: "h264",
    quality: "high",
    bitrateKbps: null,
    captureCursor: true,
    systemAudio: true,
    systemDevice: null,
    systemVolume: 1,
    mic: true,
    micDevice: null,
    micVolume: 1,
    separateTracks: true,
    diskBuffer: false,
    noiseSuppression: false,
    noiseStrength: 80,
    micMuted: false,
  },
  replayEnabled: true,
  replaySeconds: 300,
  skipSaved: true,
  shortSeconds: 30,
  clipsDir: "C:\\Users\\Player\\Videos\\GeniusClip",
  screenshotsDir: "C:\\Users\\Player\\Pictures\\GeniusClip",
  sortByGame: true,
  hotkeys: {
    saveClip: "Alt+F8",
    toggleReplay: "Alt+Shift+F8",
    screenshot: "Alt+F6",
    toggleRecording: "Alt+F7",
    saveShort: "",
    toggleMenu: "Alt+KeyX",
  },
  autostart: true,
  language: "auto",
  accent: "violet",
  overlay: { enabled: true, corner: "top-right", sound: true },
  autoUpdate: true,
  pauseOnBattery: false,
  onboarding: { clipSaved: false, menuOpened: false, dismissed: false },
};

let whatsNew: { version: string; notes: string } | null = {
  version: "0.1.1",
  notes: [
    "New",
    "- Star clips on their thumbnail or in the viewer; the gallery has a Favorites tab",
    "- A first-steps card on Home for new players",
    "- Home warns when the clips drive is almost full",
    "- Settings → System collects a problem report for the developer",
    "Fixes",
    "- Steadier thumbnails in long galleries",
  ].join("\n"),
};
/** Free space on the clips drive: low (under 5 GB) unless ?clean. */
let freeMb = 3_300;
const LOW_MB = 5_120;

const changed = () => emit("settings://changed", structuredClone(settings));

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
    encoder: "h264_nvenc",
    width: 2560,
    height: 1440,
    fps: 60,
    droppedFrames: 0,
    droppedRecent: 0,
    paused: false,
    lastError: null,
    noiseUnavailable: false,
  };
}

let micTimer: ReturnType<typeof setInterval> | undefined;

export function installMock() {
  (window as any).__GC_MOCK__ = true;
  // Screenshots for the website: /?lang=en picks the UI language.
  const params = new URLSearchParams(location.search);
  const lang = params.get("lang");
  if (lang) settings.language = lang;
  if (params.has("clean")) {
    whatsNew = null;
    freeMb = 412_000;
    settings.onboarding.dismissed = true;
  }
  (window as any).__gcMock = {
    openMenu() {
      settings.onboarding.menuOpened = true;
      changed();
    },
  };
  mockWindows("main");
  mockIPC(
    async (cmd, args: any) => {
      switch (cmd) {
        case "get_snapshot":
          return {
            settings,
            status: status(),
            monitors: [
              {
                id: "\\\\.\\DISPLAY1",
                name: "X27U",
                width: 2560,
                height: 1440,
                x: 0,
                y: 0,
                primary: true,
                hdr: false,
                adapter: "NVIDIA GeForce RTX 4090",
                vendorId: 4318,
              },
              {
                id: "\\\\.\\DISPLAY2",
                name: "VG240Y",
                width: 1920,
                height: 1080,
                x: 2560,
                y: 0,
                primary: false,
                hdr: false,
                adapter: "NVIDIA GeForce RTX 4090",
                vendorId: 4318,
              },
            ],
            audioOutputs: [
              { id: "o1", name: "Динамики (Realtek Audio)", isDefault: true },
              {
                id: "o2",
                name: "Наушники (HyperX Cloud II)",
                isDefault: false,
              },
            ],
            audioInputs: [
              { id: "i1", name: "Микрофон (HyperX Cloud II)", isDefault: true, bluetooth: false },
              { id: "i2", name: "Гарнитура (AirPods Pro)", isDefault: false, bluetooth: true },
            ],
            ramTotalMb: 16384,
            hasBattery: true,
            version: "0.1.1",
            hotkeyErrors: [],
            update: null,
            whatsNew,
            lang: navigator.language.startsWith("ru") ? "ru" : "en",
          };
        case "get_status":
          return status();
        case "estimate": {
          const s = args.settings as Settings;
          const ref = { low: 8, medium: 14, high: 20, ultra: 32 }[
            s.engine.quality
          ];
          const h = {
            native: 1440,
            p720: 720,
            p1080: 1080,
            p1440: 1440,
            p2160: 2160,
          }[s.engine.resolution];
          const w = Math.round((h * 16) / 9);
          const mbps =
            ref *
            Math.pow((w * h * s.engine.fps) / (1920 * 1080 * 60), 0.75) *
            { h264: 1, hevc: 0.7, av1: 0.65 }[s.engine.codec];
          return {
            width: w,
            height: h,
            bitrateKbps: Math.round(mbps * 1000),
            bufferMb: Math.round((mbps / 8) * s.replaySeconds),
          };
        }
        case "update_settings":
          settings = args.settings;
          return settings;
        case "set_replay_enabled":
          settings.replayEnabled = args.on;
          return;
        case "save_clip": {
          const i = media.length;
          const entry: MediaEntry = {
            path: `mock://${i}`,
            name: `Brawlhalla new ${i}`,
            game: "Brawlhalla",
            kind: "clip",
            size: 48_000_000,
            modified: Date.now(),
            duration: 10 + (i % 50),
            width: 2560,
            height: 1440,
            favorite: false,
          };
          setTimeout(() => {
            media.unshift(entry);
            freeMb -= 46;
            if (!settings.onboarding.clipSaved) {
              settings.onboarding.clipSaved = true;
              changed();
            }
            emit("engine://event", {
              type: "clipSaved",
              path: entry.path,
              seconds: entry.duration,
            });
            emit("library://changed", { path: entry.path, entry });
          }, 350);
          return;
        }
        case "delete_media": {
          const i = media.findIndex((m) => m.path === args.path);
          if (i < 0) throw "err.not-found";
          media.splice(i, 1);
          emit("library://changed", { path: args.path, removed: true });
          return null;
        }
        case "set_favorite": {
          const m = media.find((x) => x.path === args.path);
          if (!m) throw "err.not-found";
          m.favorite = args.on;
          await new Promise((r) => setTimeout(r, 120));
          emit("library://changed", { path: m.path, entry: { ...m }, removed: false });
          return { ...m };
        }
        case "disk_space":
          return { drive: "D:", freeMb, totalMb: 953_000, lowMb: LOW_MB, low: freeMb < LOW_MB };
        case "dismiss_whats_new":
          whatsNew = null;
          return null;
        case "make_report":
          await new Promise((r) => setTimeout(r, 1400));
          return "C:\\Users\\Player\\Desktop\\GeniusClip-report-2026-09-27.zip";
        case "toggle_recording":
          recording = !recording;
          recStart = Date.now();
          return;
        case "check_update": {
          const info = { version: "0.1.2", currentVersion: "0.1.1", notes: null, date: null };
          emit("update://available", info);
          return info;
        }
        case "hotkey_errors":
          return [];
        case "set_hotkeys_suspended":
          return null;
        // The in-game menu opens itself only when its window is showing.
        case "plugin:window|is_visible":
          return true;
        case "list_media":
          return media;
        case "thumbnail": {
          if (args.path === "/dev-clip.mp4") return "/dev-clip-thumb.jpg";
          const i = Number(String(args.path).split("//")[1]);
          return thumb(i, media.find((m) => m.path === args.path)?.game ?? "");
        }
        case "system_stats":
          return { cpu: 18 + Math.random() * 12, ramUsedGb: 11.4, ramTotalGb: 32, gpu: 52 + Math.random() * 20, gpuTemp: 61 };
        case "menu_close":
        case "open_in_app":
        case "take_pending_open":
          return null;
        case "mic_test": {
          clearInterval(micTimer);
          if (args.on)
            micTimer = setInterval(() => {
              const talk = Math.sin(Date.now() / 700) > 0;
              const noise = 0.02 + Math.random() * 0.01;
              const voice = 0.2 + Math.random() * 0.4;
              const k = settings.engine.noiseSuppression
                ? settings.engine.noiseStrength / 100
                : 0;
              emit("mic://level", [
                talk ? voice : noise,
                talk ? voice * 0.95 : noise * (1 - k * 0.97),
              ]);
            }, 50);
          return null;
        }
        case "clip_audio": {
          if (args.path !== "/dev-clip.mp4")
            return { titles: [], mix: false, files: [], peaks: [] };
          const peaks = await fetch("/dev-clip.peaks.json").then((r) =>
            r.json(),
          );
          const titles = ["Game + Mic", "Game", "Mic"];
          return {
            titles,
            mix: true,
            files: titles.map((_, k) => `/dev-clip_${k}.m4a`),
            peaks,
          };
        }
        case "trim_media":
          await new Promise((r) => setTimeout(r, 600));
          console.log("[mock] trim_media", args);
          return null;
        case "plugin:event|listen":
          return Math.floor(Math.random() * 1e6);
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  setInterval(() => {
    buffer += 1;
    emit("engine://status", status());
  }, 1000);
}
