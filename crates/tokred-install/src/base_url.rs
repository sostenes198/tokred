//! `env.ANTHROPIC_BASE_URL` in Claude Code's `settings.json`: the setting that points Claude Code
//! at the local proxy.
//!
//! The pure functions work on [`serde_json::Value`]s like [`crate::settings`] does: they touch
//! only that one key and keep everything else in its original order. The file functions never
//! write a backup (they run on every launch of `claude`; the backup `configure` makes must keep
//! holding the user's original file) and never write when nothing changes.

use std::path::Path;

use serde_json::{Map, Value};
use tokred_core::FileSystem;

use crate::configure::{read_settings, serialize_settings, settings_path};
use crate::error::{InstallError, MergeError};

/// The variable Claude Code reads the API address from.
pub const BASE_URL_KEY: &str = "ANTHROPIC_BASE_URL";

/// The value of `env.ANTHROPIC_BASE_URL`, when it is a string.
#[must_use]
pub fn current(settings: &Value) -> Option<&str> {
    settings
        .get("env")?
        .get(BASE_URL_KEY)?
        .as_str()
        .filter(|url| !url.is_empty())
}

/// Sets `env.ANTHROPIC_BASE_URL` to `url`, creating `env` when it is missing. Other keys of
/// `env`, and the position of the key when it already exists, are kept. Running it twice gives
/// the same result as running it once.
pub fn set(mut settings: Value, url: &str) -> Result<Value, MergeError> {
    let root = settings.as_object_mut().ok_or(MergeError::RootNotObject)?;
    let env = root
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(MergeError::EnvNotObject)?;
    env.insert(BASE_URL_KEY.to_owned(), Value::String(url.to_owned()));
    Ok(settings)
}

/// Removes `env.ANTHROPIC_BASE_URL`. `env` is dropped only when removing the key is what
/// emptied it. Anything that is not the expected shape is left exactly as it is.
#[must_use]
pub fn remove(mut settings: Value) -> Value {
    let Some(root) = settings.as_object_mut() else {
        return settings;
    };
    let Some(env) = root.get_mut("env").and_then(Value::as_object_mut) else {
        return settings;
    };
    if env.shift_remove(BASE_URL_KEY).is_some() && env.is_empty() {
        root.shift_remove("env");
    }
    settings
}

/// The value of `env.ANTHROPIC_BASE_URL` in `<claude_dir>/settings.json` (`None` when the file
/// or the key does not exist).
pub fn read_base_url(
    fs: &dyn FileSystem,
    claude_dir: &Path,
) -> Result<Option<String>, InstallError> {
    let (settings, _) = read_settings(fs, &settings_path(claude_dir))?;
    Ok(current(&settings).map(str::to_owned))
}

/// Makes `env.ANTHROPIC_BASE_URL` in `<claude_dir>/settings.json` equal to `desired` (`None`
/// removes it). Returns whether the file was written; it is not when it already says so. A file
/// that is not valid JSON is never written.
pub fn write_base_url(
    fs: &dyn FileSystem,
    claude_dir: &Path,
    desired: Option<&str>,
) -> Result<bool, InstallError> {
    let path = settings_path(claude_dir);
    let (current, _) = read_settings(fs, &path)?;
    let updated = match desired {
        Some(url) => set(current.clone(), url).map_err(|source| InstallError::Merge {
            path: path.clone(),
            source,
        })?,
        None => remove(current.clone()),
    };
    if updated == current {
        return Ok(false);
    }
    fs.create_dir_all(claude_dir)?;
    fs.write_atomic(&path, serialize_settings(&updated)?.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_value_is_read_only_when_it_is_a_non_empty_string() {
        assert_eq!(
            current(&json!({"env": {BASE_URL_KEY: "http://127.0.0.1:1"}})),
            Some("http://127.0.0.1:1")
        );
        for none in [
            json!({}),
            json!({"env": 3}),
            json!({"env": {}}),
            json!({"env": {BASE_URL_KEY: 3}}),
            json!({"env": {BASE_URL_KEY: ""}}),
        ] {
            assert_eq!(current(&none), None, "{none}");
        }
    }

    #[test]
    fn setting_adds_the_key_and_keeps_everything_else() {
        let set_in = |settings: Value| set(settings, "http://127.0.0.1:1029").unwrap();
        assert_eq!(
            set_in(json!({})),
            json!({"env": {BASE_URL_KEY: "http://127.0.0.1:1029"}})
        );
        let merged = set_in(json!({"model": "opus", "env": {"A": "1"}, "hooks": {}}));
        assert_eq!(merged["model"], "opus");
        assert_eq!(merged["env"]["A"], "1");
        assert_eq!(merged["env"][BASE_URL_KEY], "http://127.0.0.1:1029");
        let order: Vec<_> = merged.as_object().unwrap().keys().collect();
        assert_eq!(order, ["model", "env", "hooks"], "no key moves");
        assert_eq!(set_in(merged.clone()), merged, "twice is once");

        let replaced = set(
            json!({"env": {"A": "1", BASE_URL_KEY: "http://127.0.0.1:1029", "B": "2"}}),
            "http://127.0.0.1:1029",
        )
        .unwrap();
        let keys: Vec<_> = replaced["env"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["A", BASE_URL_KEY, "B"], "the key keeps its place");
    }

    #[test]
    fn setting_refuses_shapes_it_cannot_write_into() {
        assert_eq!(set(json!([1]), "u"), Err(MergeError::RootNotObject));
        assert_eq!(set(json!({"env": 3}), "u"), Err(MergeError::EnvNotObject));
    }

    #[test]
    fn removing_drops_env_only_when_it_emptied_it() {
        assert_eq!(remove(json!({"env": {BASE_URL_KEY: "u"}})), json!({}));
        assert_eq!(
            remove(json!({"env": {"A": "1", BASE_URL_KEY: "u"}})),
            json!({"env": {"A": "1"}})
        );
        for untouched in [
            json!({"env": {}}),
            json!({"env": {"A": "1"}}),
            json!({"env": 3}),
            json!({"model": "opus"}),
            json!([1]),
        ] {
            assert_eq!(remove(untouched.clone()), untouched);
        }
    }
}
