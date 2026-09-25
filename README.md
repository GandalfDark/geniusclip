# GeniusClip

Запись последних минут игры/экрана по горячей клавише (как NVIDIA Instant Replay).

## Устройство

- `crates/engine` — движок на Rust: захват экрана (DXGI Desktop Duplication) →
  конвертация BGRA→NV12 на GPU (D3D11 Video Processor) → аппаратное кодирование
  (NVENC / AMF / Media Foundation) → кольцевой буфер в памяти → MP4 без
  перекодирования. Звук: WASAPI (система + микрофон) → AAC, до 3 дорожек.
- `src-tauri` — приложение Tauri 2: трей, горячие клавиши, настройки, автозапуск,
  обновления, оверлей-уведомления.
- `src` — интерфейс (SvelteKit).
- `third_party/ffmpeg` — минимальная LGPL-сборка FFmpeg (не в git).

## Сборка

```powershell
# один раз: FFmpeg (собирается в GitHub Actions workflow "Build FFmpeg")
./scripts/fetch-ffmpeg.ps1
npm install
npm run tauri dev
```

Нужны: Rust (MSVC), Node.js, Visual Studio Build Tools, LLVM (libclang для bindgen).

Проверка движка без интерфейса:

```powershell
cargo run -p geniusclip-engine --example replay -- 8 target/replay-test
```
