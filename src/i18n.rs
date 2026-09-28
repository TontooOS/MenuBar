//! Translations via Accessibility (`lang/en_us.json`, `lang/de_de.json`).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::Accessibility::{LangFile, LangStore};

static LOCALE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// System language: `de_de` when locale starts with `de`, else `en_us`.
pub fn sys_lang() -> String {
  for key in ["LANGUAGE", "LANG", "LC_ALL"] {
    if let Ok(value) = std::env::var(key) {
      if value.is_empty() {
        continue;
      }
      let lower = value.to_lowercase();
      if lower.starts_with("de") {
        return "de_de".to_string();
      }
      return "en_us".to_string();
    }
  }
  if let Ok(content) = std::fs::read_to_string("/etc/locale.conf") {
    for line in content.lines() {
      if let Some(lang) = line.trim().strip_prefix("LANG=") {
        if lang.to_lowercase().starts_with("de") {
          return "de_de".to_string();
        }
        return "en_us".to_string();
      }
    }
  }
  "en_us".to_string()
}

fn lang_dirs() -> Vec<PathBuf> {
  let mut dirs = Vec::new();
  if let Ok(env) = std::env::var("MENUBAR_LANG_DIR") {
    if !env.is_empty() {
      dirs.push(PathBuf::from(env));
    }
  }
  dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lang"));
  if let Ok(cwd) = std::env::current_dir() {
    dirs.push(cwd.join("lang"));
  }
  if let Ok(exe) = std::env::current_exe() {
    if let Some(parent) = exe.parent() {
      dirs.push(parent.join("lang"));
      if let Some(grand) = parent.parent() {
        dirs.push(grand.join("lang"));
        dirs.push(grand.join("Resources").join("lang"));
      }
    }
  }
  dirs.push(PathBuf::from("/usr/share/tontoo/menubar/lang"));
  dirs
}

/// Load strings for the detected locale. Safe to call multiple times.
pub fn init_i18n() {
  if LOCALE.get().is_some() {
    return;
  }
  let locale = sys_lang();
  let mut files: Vec<LangFile> = Vec::new();
  for dir in lang_dirs() {
    for code in ["en_us", "de_de"] {
      let path = dir.join(format!("{code}.json"));
      if let Ok(file) = LangFile::from_file(&path) {
        if !files.iter().any(|f| f.lang == file.lang) {
          files.push(file);
        }
      }
    }
  }
  if !files.is_empty() {
    if let Err(e) = LangStore::init(files, Some("en_us".to_string())) {
      eprintln!("[menubar] i18n init failed: {e}");
    }
  }
  let _ = LOCALE.set(locale);
}

/// Translate `key` with the system language.
pub fn trk(key: &str) -> String {
  init_i18n();
  let locale = LOCALE.get().cloned().unwrap_or_else(|| "en_us".to_string());
  LangStore::instance()
    .t(&locale, key, None)
    .unwrap_or_else(|| key.to_string())
}

/// Translate `key` with `%name%` placeholders from `args`.
pub fn trk_args(key: &str, args: &[(&str, &str)]) -> String {
  init_i18n();
  let locale = LOCALE.get().cloned().unwrap_or_else(|| "en_us".to_string());
  let map: HashMap<String, String> = args
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
  LangStore::instance()
    .t(&locale, key, Some(&map))
    .unwrap_or_else(|| key.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn lang_files_load() {
    init_i18n();
    // Packaged files must resolve (cargo test runs at the crate root).
    assert_eq!(trk("menu.about"), "About This Maschine");
  }

  #[test]
  fn missing_key_returns_key() {
    assert_eq!(trk("no.such.key"), "no.such.key");
  }
}
