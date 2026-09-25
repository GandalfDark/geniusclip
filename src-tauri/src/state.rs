use crate::library::Library;
use crate::settings::Settings;
use crate::updates::UpdateInfo;
use geniusclip_engine::{game::AppInfo, Engine};
use parking_lot::{Mutex, RwLock};
use std::time::Instant;

pub struct AppState {
    pub engine: Engine,
    pub settings: RwLock<Settings>,
    pub library: Library,
    /// Most recent non-desktop foreground app, used to name clips saved
    /// from the tray/UI while the game is not in focus.
    pub last_game: Mutex<Option<(AppInfo, Instant)>>,
    pub hotkey_errors: Mutex<Vec<String>>,
    pub update: Mutex<Option<UpdateInfo>>,
}
