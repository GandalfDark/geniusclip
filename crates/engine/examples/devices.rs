//! Lists audio devices with their Bluetooth flag.
fn main() -> anyhow::Result<()> {
    for capture in [false, true] {
        for d in geniusclip_engine::list_audio_devices(capture)? {
            println!("{} {}{}{}", if capture { "in " } else { "out" }, d.name, if d.is_default { " (default)" } else { "" }, if d.bluetooth { " [Bluetooth]" } else { "" });
        }
    }
    Ok(())
}
