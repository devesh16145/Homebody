//! Each app's own icon, so people recognise their apps at a glance.
//!
//! A windowed app is its own Mac app. A command-line agent has no icon of its own, so the icon of
//! its maker's Mac app is used when that is installed (Claude for Claude Code, Cursor for the
//! Cursor CLI, and so on). Without one the interface draws a letter tile instead.
//!
//! The icon is drawn by macOS itself (NSWorkspace, through the scripting bridge that ships with
//! every Mac: no extra libraries), at 128 px, and kept in the data folder. Nothing leaves the Mac.

use crate::config::AppConfig;
use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Command-line agents and the Mac app whose icon stands for them, in order of preference.
const MAKERS: &[(&str, &[&str])] = &[
    ("claude-code", &["Claude.app"]),
    ("codex", &["Codex.app", "ChatGPT.app"]),
    ("cursor-agent", &["Cursor.app"]),
    ("ollama", &["Ollama.app"]),
    ("gemini", &["Gemini.app"]),
    ("llama-cpp", &[]),
];

/// Draws a bundle's icon into a 128 px PNG: osascript -l JavaScript <this> <bundle> <out.png>
const DRAW: &str = r#"ObjC.import('AppKit');
function run(argv) {
  var img = $.NSWorkspace.sharedWorkspace.iconForFile(argv[0]);
  var px = 128;
  var rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(null, px, px, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
  $.NSGraphicsContext.saveGraphicsState;
  $.NSGraphicsContext.setCurrentContext($.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep));
  img.drawInRectFromRectOperationFraction($.NSMakeRect(0, 0, px, px), $.NSZeroRect, $.NSCompositingOperationSourceOver, 1);
  $.NSGraphicsContext.restoreGraphicsState;
  var png = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $());
  return png.writeToFileAtomically(argv[1], true) ? 'ok' : 'failed';
}"#;

fn app_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        v.push(home.join("Applications"));
    }
    v
}

/// The Mac app whose icon stands for `app`, if one is installed.
pub fn source(app: &AppConfig) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None; // icons are drawn by macOS; elsewhere the interface uses letter tiles
    }
    if app.binary.ends_with(".app") && Path::new(&app.binary).is_dir() {
        return Some(PathBuf::from(&app.binary));
    }
    let names = MAKERS.iter().find(|(id, _)| *id == app.id).map(|(_, n)| *n)?;
    names.iter().flat_map(|n| app_dirs().into_iter().map(move |d| d.join(n))).find(|p| p.is_dir())
}

fn cache_path(dir: &Path, app: &AppConfig, src: &Path) -> PathBuf {
    // A different source (the maker's app was moved, or the user picked another) gets a new file.
    let tag: u32 = src.to_string_lossy().bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    dir.join(format!("{}-{:08x}.png", app.id, tag))
}

/// The icon as a `data:` URL, drawing and caching it on first use. `None` when there is none.
pub async fn icon(app: &AppConfig, dir: &Path) -> Option<String> {
    let src = source(app)?;
    let out = cache_path(dir, app, &src);
    if !out.is_file() {
        std::fs::create_dir_all(dir).ok()?;
        let tmp = out.with_extension("tmp.png");
        let run = tokio::process::Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", DRAW])
            .arg(&src)
            .arg(&tmp)
            .output();
        let done = tokio::time::timeout(Duration::from_secs(10), run).await.ok()?.ok()?;
        if !done.status.success() || !tmp.is_file() {
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
        std::fs::rename(&tmp, &out).ok()?;
    }
    let bytes = std::fs::read(&out).ok()?;
    (bytes.len() < 512 * 1024).then(|| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, binary: &str) -> AppConfig {
        let mut a = crate::config::Config::fresh().apps.remove(0);
        a.id = id.into();
        a.binary = binary.into();
        a
    }

    #[test]
    fn an_agent_without_a_makers_app_has_no_source() {
        assert!(source(&app("someone-elses-tool", "mytool")).is_none());
        assert!(source(&app("llama-cpp", "llama-server")).is_none());
    }

    #[test]
    fn a_windowed_app_is_its_own_source_only_when_it_exists() {
        assert!(source(&app("w", "/Applications/No Such App.app")).is_none());
        if Path::new("/System/Applications/TextEdit.app").is_dir() {
            assert_eq!(source(&app("w", "/System/Applications/TextEdit.app")).unwrap(), PathBuf::from("/System/Applications/TextEdit.app"));
        }
    }

    #[test]
    fn a_moved_source_gets_a_new_cache_file() {
        let a = app("claude-code", "claude");
        let d = Path::new("/tmp/x");
        assert_ne!(cache_path(d, &a, Path::new("/Applications/Claude.app")), cache_path(d, &a, Path::new("/Users/me/Applications/Claude.app")));
    }
}
