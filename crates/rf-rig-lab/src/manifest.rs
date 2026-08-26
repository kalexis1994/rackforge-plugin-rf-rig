//! Reads the few manifest fields the generated metadata has to agree with.
//!
//! RackForge refuses to install a package whose runtime descriptor disagrees
//! with its manifest, and it says so in a dialog rather than on the console.
//! Generating the descriptor *from* the manifest is how this project makes that
//! failure impossible instead of memorable.

use std::path::Path;

pub struct PackageIdentity {
    pub id: String,
    pub version: String,
    pub state_version: u32,
}

/// Minimal reader for the top-level table of `rackforge-plugin.toml`. It stops
/// at the first section header, which is where every field it needs lives.
pub fn read(path: &Path) -> Result<PackageIdentity, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut id = None;
    let mut version = None;
    let mut state_version = None;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match key {
            "id" => id = Some(value.to_owned()),
            "version" => version = Some(value.to_owned()),
            "state_version" => state_version = value.parse::<u32>().ok(),
            _ => {}
        }
    }

    Ok(PackageIdentity {
        id: id.ok_or("the manifest has no id")?,
        version: version.ok_or("the manifest has no version")?,
        state_version: state_version.ok_or("the manifest has no state_version")?,
    })
}
