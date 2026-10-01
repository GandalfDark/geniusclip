//! Auto clips in Counter-Strike 2 and Dota 2, through Valve's Game State
//! Integration: a small config file in the game's folder makes the game post
//! its state (kills, rounds, deaths) as JSON to a local address, and the
//! events the user picked save a clip of the moment.
//!
//! The address is 127.0.0.1 only, and every post has to carry the token
//! written into the config, so nothing else on the network can trigger clips.
//! Clips of a streak (a multi-kill, an ace, a rampage) are saved once it is
//! over, covering it from its first kill.

use crate::settings::AutoClips;
use crate::state::AppState;
use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const FIRST_PORT: u16 = 47823;
const CONFIG_NAME: &str = "gamestate_integration_geniusclip.cfg";
/// Dota counts kills as one multi-kill while each follows within this.
const DOTA_STREAK: Duration = Duration::from_secs(18);
/// A single kill's clip: the lead-up, and a moment after it.
const KILL_BEFORE: Duration = Duration::from_secs(12);
const KILL_AFTER: Duration = Duration::from_secs(3);
/// Before the first kill of a streak.
const STREAK_LEAD: Duration = Duration::from_secs(5);
const MAX_CLIP: Duration = Duration::from_secs(120);

static PORT: OnceLock<u16> = OnceLock::new();
static GAMES: Mutex<Games> = Mutex::new(Games::new());
static QUEUE: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static WAKE: Condvar = Condvar::new();

#[derive(Clone, Copy, PartialEq, Debug)]
enum Game {
    Cs2,
    Dota,
}

/// A clip to save at `due`, starting at `from`. A newer one of the same
/// `key` replaces it (more kills in a row make one longer clip).
struct Pending {
    key: (Game, &'static str),
    due: Instant,
    from: Instant,
}

#[derive(Default)]
struct Cs2 {
    seen: Option<Instant>,
    round_kills: u64,
    first_kill: Option<Instant>,
    phase: String,
    /// The player's team, from their own state (after dying, the game sends
    /// the state of whoever they watch).
    team: Option<String>,
}

#[derive(Default)]
struct Dota {
    seen: Option<Instant>,
    kills: Option<u64>,
    deaths: Option<u64>,
    streak: Vec<Instant>,
}

struct Games {
    cs2: Option<Cs2>,
    dota: Option<Dota>,
}

impl Games {
    const fn new() -> Games {
        Games { cs2: None, dota: None }
    }
}

/// What the settings page shows for a game.
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GameStatus {
    /// The game is installed (a Steam library has it).
    found: bool,
    /// Our config is in the game's folder.
    installed: bool,
    /// The game has sent its state lately.
    connected: bool,
    /// Writing the config failed (no access to the folder).
    failed: bool,
}

#[derive(Serialize, Default, Clone)]
pub struct Status {
    cs2: GameStatus,
    dota: GameStatus,
}

static FAILED: Mutex<(bool, bool)> = Mutex::new((false, false));

/// Listens for the games and writes their configs. Once, at startup.
pub fn start(app: &AppHandle) {
    let listener = (FIRST_PORT..FIRST_PORT + 8).find_map(|p| TcpListener::bind(("127.0.0.1", p)).ok());
    let Some(listener) = listener else {
        log::warn!("auto clips: no free local port");
        return;
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(FIRST_PORT);
    let _ = PORT.set(port);
    let h = app.clone();
    let _ = std::thread::Builder::new().name("gc-gsi".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Err(e) = handle(&h, stream) {
                log::debug!("auto clips: {e}");
            }
        }
    });
    let h = app.clone();
    let _ = std::thread::Builder::new().name("gc-autoclip".into()).spawn(move || saver(h));
    sync(app);
}

/// Writes or removes the games' configs to match the settings.
pub fn sync(app: &AppHandle) {
    let Some(&port) = PORT.get() else { return };
    let s = app.state::<AppState>().settings.read().auto_clips.clone();
    let mut failed = FAILED.lock();
    failed.0 = apply(cs2_dir(), s.cs2, &cs2_config(port, &s.token)).is_err();
    failed.1 = apply(dota_dir(), s.dota, &dota_config(port, &s.token)).is_err();
}

fn apply(dir: Option<PathBuf>, on: bool, config: &str) -> std::io::Result<()> {
    let Some(dir) = dir else { return Ok(()) };
    let file = dir.join(CONFIG_NAME);
    if on {
        if std::fs::read_to_string(&file).ok().as_deref() != Some(config) {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&file, config).inspect_err(|e| log::warn!("auto clips: {}: {e}", file.display()))?;
        }
    } else if file.exists() {
        std::fs::remove_file(&file)?;
    }
    Ok(())
}

/// The game sent its state lately (it posts every few seconds while running).
fn recent(seen: Option<Instant>) -> bool {
    seen.is_some_and(|t| t.elapsed() < Duration::from_secs(90))
}

pub fn status() -> Status {
    let games = GAMES.lock();
    let failed = *FAILED.lock();
    let game = |dir: Option<PathBuf>, seen: Option<Instant>, failed: bool| GameStatus {
        found: dir.is_some(),
        installed: dir.is_some_and(|d| d.join(CONFIG_NAME).exists()),
        connected: recent(seen),
        failed,
    };
    Status {
        cs2: game(cs2_dir(), games.cs2.as_ref().and_then(|g| g.seen), failed.0),
        dota: game(dota_dir(), games.dota.as_ref().and_then(|g| g.seen), failed.1),
    }
}

// ---------------------------------------------------------------------------
// Where the games are

/// Every Steam library folder (the main one and those added in Steam).
fn steam_libraries() -> Vec<PathBuf> {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = vec![0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Valve\Steam"),
            &HSTRING::from("SteamPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if ok.is_err() {
        return Vec::new();
    }
    let steam = PathBuf::from(String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)]));
    let mut libs = vec![steam.clone()];
    if let Ok(vdf) = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for line in vdf.lines() {
            let parts: Vec<&str> = line.split('"').collect();
            // 	"path"		"D:\\SteamLibrary"
            if parts.len() >= 4 && parts[1] == "path" {
                libs.push(PathBuf::from(parts[3].replace("\\\\", "\\")));
            }
        }
    }
    libs.dedup();
    libs
}

fn game_dir(folder: &str, cfg: &[&str]) -> Option<PathBuf> {
    steam_libraries().into_iter().find_map(|lib| {
        let root = lib.join("steamapps").join("common").join(folder);
        root.is_dir().then(|| cfg.iter().fold(root, |p, part| p.join(part)))
    })
}

fn cs2_dir() -> Option<PathBuf> {
    game_dir("Counter-Strike Global Offensive", &["game", "csgo", "cfg"])
}

fn dota_dir() -> Option<PathBuf> {
    game_dir("dota 2 beta", &["game", "dota", "cfg", "gamestate_integration"])
}

fn cs2_config(port: u16, token: &str) -> String {
    format!(
        "\"GeniusClip\"\n{{\n  \"uri\" \"http://127.0.0.1:{port}/cs2\"\n  \"timeout\" \"5.0\"\n  \"buffer\" \"0.1\"\n  \"throttle\" \"0.1\"\n  \"heartbeat\" \"30.0\"\n  \"auth\"\n  {{\n    \"token\" \"{token}\"\n  }}\n  \"data\"\n  {{\n    \"provider\" \"1\"\n    \"round\" \"1\"\n    \"player_id\" \"1\"\n    \"player_state\" \"1\"\n  }}\n}}\n"
    )
}

fn dota_config(port: u16, token: &str) -> String {
    format!(
        "\"GeniusClip\"\n{{\n  \"uri\" \"http://127.0.0.1:{port}/dota\"\n  \"timeout\" \"5.0\"\n  \"buffer\" \"0.1\"\n  \"throttle\" \"0.1\"\n  \"heartbeat\" \"30.0\"\n  \"auth\"\n  {{\n    \"token\" \"{token}\"\n  }}\n  \"data\"\n  {{\n    \"provider\" \"1\"\n    \"map\" \"1\"\n    \"player\" \"1\"\n  }}\n}}\n"
    )
}

// ---------------------------------------------------------------------------
// What the games send

fn handle(app: &AppHandle, stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut length = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; length.min(1 << 20)];
    reader.read_exact(&mut body)?;
    let mut out = stream;
    out.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;

    let Ok(json) = serde_json::from_slice::<Value>(&body) else { return Ok(()) };
    let s = app.state::<AppState>().settings.read().auto_clips.clone();
    if json["auth"]["token"].as_str() != Some(s.token.as_str()) {
        return Ok(());
    }
    match path.as_str() {
        "/cs2" => on_cs2(&s, &json),
        "/dota" => on_dota(&s, &json),
        _ => {}
    }
    Ok(())
}

fn wants(events: &[String], e: &str) -> bool {
    events.iter().any(|x| x == e)
}

fn on_cs2(s: &AutoClips, json: &Value) {
    let now = Instant::now();
    let mut games = GAMES.lock();
    let g = games.cs2.get_or_insert_with(Cs2::default);
    if !recent(g.seen) {
        log::info!("auto clips: Counter-Strike 2 is sending its state");
    }
    g.seen = Some(now);
    if !s.cs2 {
        return;
    }
    // Only the player's own state (not someone watched after dying).
    let me = json["provider"]["steamid"].as_str();
    let player = &json["player"];
    let own = me.is_some() && player["steamid"].as_str() == me;
    let phase = json["round"]["phase"].as_str().unwrap_or("").to_string();
    if own {
        let kills = player["state"]["round_kills"].as_u64().unwrap_or(0);
        if kills > g.round_kills {
            g.first_kill.get_or_insert(now);
            if wants(&s.cs2_events, "kill") {
                schedule((Game::Cs2, "kill"), now + KILL_AFTER, now - KILL_BEFORE);
            }
        }
        g.round_kills = kills;
        if let Some(t) = player["team"].as_str() {
            g.team = Some(t.to_string());
        }
    }
    // The round is over: its streak and its win.
    if phase == "over" && g.phase != "over" {
        let from = g.first_kill.map(|t| t - STREAK_LEAD);
        let ace = g.round_kills >= 5 && wants(&s.cs2_events, "ace");
        let multi = g.round_kills >= 3 && wants(&s.cs2_events, "multikill");
        if let (Some(from), true) = (from, ace || multi) {
            schedule((Game::Cs2, "streak"), now + KILL_AFTER, from);
        }
        let team = g.team.as_deref();
        if team.is_some() && json["round"]["win_team"].as_str() == team && wants(&s.cs2_events, "roundWin") {
            schedule((Game::Cs2, "round"), now + KILL_AFTER, now - Duration::from_secs(20));
        }
    }
    if phase == "freezetime" || (phase == "live" && g.phase == "over") {
        g.round_kills = 0;
        g.first_kill = None;
    }
    g.phase = phase;
}

fn on_dota(s: &AutoClips, json: &Value) {
    let now = Instant::now();
    let mut games = GAMES.lock();
    let g = games.dota.get_or_insert_with(Dota::default);
    if !recent(g.seen) {
        log::info!("auto clips: Dota 2 is sending its state");
    }
    g.seen = Some(now);
    if !s.dota {
        return;
    }
    let player = &json["player"];
    if let Some(kills) = player["kills"].as_u64() {
        // A new match, or the first message of one: only a rise counts.
        if g.kills.is_some_and(|k| kills > k) {
            if g.streak.last().is_some_and(|t| now - *t > DOTA_STREAK) {
                g.streak.clear();
            }
            g.streak.push(now);
            if wants(&s.dota_events, "kill") {
                schedule((Game::Dota, "kill"), now + KILL_AFTER, now - KILL_BEFORE);
            }
            // Judged once the streak can't grow any more.
            let n = g.streak.len();
            let big = (n >= 5 && wants(&s.dota_events, "rampage")) || (n >= 3 && wants(&s.dota_events, "multikill"));
            if big {
                schedule((Game::Dota, "streak"), now + DOTA_STREAK, g.streak[0] - STREAK_LEAD);
            }
        }
        g.kills = Some(kills);
    }
    if let Some(deaths) = player["deaths"].as_u64() {
        if g.deaths.is_some_and(|d| deaths > d) && wants(&s.dota_events, "death") {
            schedule((Game::Dota, "death"), now + KILL_AFTER, now - Duration::from_secs(15));
        }
        g.deaths = Some(deaths);
    }
}

// ---------------------------------------------------------------------------
// Saving

fn schedule(key: (Game, &'static str), due: Instant, from: Instant) {
    let mut q = QUEUE.lock();
    match q.iter_mut().find(|p| p.key == key) {
        // One clip for kills in a row: it starts at the first and ends after the last.
        Some(p) => {
            p.due = p.due.max(due);
            p.from = p.from.min(from);
        }
        None => q.push(Pending { key, due, from }),
    }
    WAKE.notify_one();
}

fn saver(app: AppHandle) {
    let mut q = QUEUE.lock();
    loop {
        let now = Instant::now();
        let due: Vec<Pending> = {
            let (ready, wait): (Vec<_>, Vec<_>) = q.drain(..).partition(|p| p.due <= now);
            *q = wait;
            ready
        };
        if !due.is_empty() {
            drop(q);
            // Events due together (a kill that is also a multi-kill's last)
            // make one clip, the longest.
            if let Some(p) = due.iter().min_by_key(|p| p.from) {
                let secs = (now - p.from).min(MAX_CLIP).as_secs().max(8) as u32;
                log::info!("auto clip ({:?} {}): {secs} s", p.key.0, p.key.1);
                crate::actions::save_seconds(&app, secs);
            }
            q = QUEUE.lock();
            continue;
        }
        match q.iter().map(|p| p.due).min() {
            Some(next) => {
                WAKE.wait_until(&mut q, next);
            }
            None => WAKE.wait(&mut q),
        }
    }
}

/// A token the games send back with every post.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for _ in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cs2(steamid: &str, kills: u64, phase: &str, win: &str) -> Value {
        json!({
            "provider": { "steamid": "me" },
            "player": { "steamid": steamid, "team": "CT", "state": { "round_kills": kills } },
            "round": { "phase": phase, "win_team": win },
        })
    }

    fn queued() -> Vec<(Game, &'static str)> {
        QUEUE.lock().iter().map(|p| p.key).collect()
    }

    // One test: the detection state is global.
    #[test]
    fn events_become_clips() {
        let s = AutoClips { cs2: true, cs2_events: vec!["multikill".into(), "roundWin".into()], dota: true, dota_events: vec!["multikill".into(), "death".into()], ..Default::default() };

        // CS2: three kills, then death (watching a teammate, who gets more), round won.
        on_cs2(&s, &cs2("me", 0, "live", ""));
        for k in 1..=3 {
            on_cs2(&s, &cs2("me", k, "live", ""));
        }
        assert!(queued().is_empty(), "single kills aren't wanted");
        on_cs2(&s, &cs2("mate", 4, "live", ""));
        on_cs2(&s, &cs2("mate", 4, "over", "CT"));
        let q = queued();
        assert!(q.contains(&(Game::Cs2, "streak")), "{q:?}");
        assert!(q.contains(&(Game::Cs2, "round")), "{q:?}");
        // Next round starts clean.
        on_cs2(&s, &cs2("me", 0, "freezetime", ""));
        assert_eq!(GAMES.lock().cs2.as_ref().unwrap().round_kills, 0);
        QUEUE.lock().clear();

        // Dota: the first message only sets the counts; two kills aren't a multi-kill.
        on_dota(&s, &json!({ "player": { "kills": 4, "deaths": 1 } }));
        on_dota(&s, &json!({ "player": { "kills": 5, "deaths": 1 } }));
        on_dota(&s, &json!({ "player": { "kills": 6, "deaths": 1 } }));
        assert!(queued().is_empty());
        on_dota(&s, &json!({ "player": { "kills": 7, "deaths": 1 } }));
        assert_eq!(queued(), vec![(Game::Dota, "streak")]);
        on_dota(&s, &json!({ "player": { "kills": 7, "deaths": 2 } }));
        assert!(queued().contains(&(Game::Dota, "death")));
        QUEUE.lock().clear();
    }

    #[test]
    fn configs_carry_port_and_token() {
        let c = cs2_config(47823, "abc");
        assert!(c.contains("\"uri\" \"http://127.0.0.1:47823/cs2\"") && c.contains("\"token\" \"abc\""));
        assert!(dota_config(47824, "x").contains("/dota"));
    }
}
