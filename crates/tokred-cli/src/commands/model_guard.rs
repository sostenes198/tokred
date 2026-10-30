//! `tokred model-guard`: the mode and the limit of the `model-guard` hook.
//!
//! The hook reads `tokred.toml` on every run, so a change applies at once: nothing is restarted.

use anyhow::{Context, Result};
use tokred_core::config::Mode;

use super::advice::set_key;
use super::housekeeping::load_config;
use crate::args::ModelGuardCommand;
use crate::env::Env;
use crate::io::Io;

pub fn run(env: &Env, io: &mut Io, command: &ModelGuardCommand) -> Result<()> {
    match command {
        ModelGuardCommand::On => set_mode(
            env,
            io,
            "on",
            "model-guard asks you to confirm a model switch that throws away a warm prompt cache.",
        ),
        ModelGuardCommand::Warn => set_mode(
            env,
            io,
            "warn",
            "model-guard says what it costs to switch the model and lets the switch go ahead.",
        ),
        ModelGuardCommand::Off => set_mode(env, io, "off", "model-guard does nothing."),
        ModelGuardCommand::Limit { tokens } => set_limit(env, io, *tokens),
        ModelGuardCommand::Status => status(env, io),
    }
}

/// Writes `key = value` of `section` to `tokred.toml`, which is created when it is missing.
fn write_key(env: &Env, section: &str, key: &str, value: toml::Value) -> Result<()> {
    let path = env.data.config_path();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, set_key(&existing, section, key, value)?)
        .with_context(|| format!("could not write {}", path.display()))
}

fn set_mode(env: &Env, io: &mut Io, value: &str, note: &str) -> Result<()> {
    write_key(
        env,
        "modes",
        "model_guard",
        toml::Value::String(value.to_owned()),
    )?;
    write!(io.out(), "[modes] model_guard = \"{value}\"\n{note}\n")?;
    Ok(())
}

fn set_limit(env: &Env, io: &mut Io, tokens: u64) -> Result<()> {
    let value = i64::try_from(tokens)
        .map_err(|_| anyhow::anyhow!("{tokens} tokens is too large for tokred.toml"))?;
    write_key(
        env,
        "model_guard",
        "min_context_tokens",
        toml::Value::Integer(value),
    )?;
    write!(
        io.out(),
        "[model_guard] min_context_tokens = {tokens}\nA model switch is reported from {tokens} tokens of context.\n"
    )?;
    Ok(())
}

/// The name a mode has for this hook: `on` is what the others call `enforce`.
const fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Off => "off",
        Mode::Warn => "warn",
        Mode::Enforce => "on",
    }
}

fn status(env: &Env, io: &mut Io) -> Result<()> {
    let config = load_config(env);
    write!(
        io.out(),
        "tokred model-guard\nMode:   {}\nLimit:  {} tokens of context\nNeeds Claude Code 2.1.251 or later; applies at once, nothing to restart.\n",
        mode_name(config.modes.model_guard),
        config.model_guard.min_context_tokens,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokred_core::config::Mode;

    use super::super::housekeeping::load_config;
    use super::super::testing::Sandbox;
    use super::*;
    use crate::args::ModelGuardCommand;
    use crate::io::testing::scripted;

    fn configured() -> Sandbox {
        let sb = Sandbox::new();
        sb.configure();
        sb
    }

    fn run_command(sb: &Sandbox, command: &ModelGuardCommand) -> String {
        let (mut io, out) = scripted(false, "");
        run(&sb.env, &mut io, command).unwrap();
        out.text()
    }

    #[test]
    fn on_warn_and_off_set_the_mode_and_say_what_it_does() {
        let sb = configured();
        for (command, mode, words) in [
            (ModelGuardCommand::On, Mode::Enforce, "asks you to confirm"),
            (ModelGuardCommand::Off, Mode::Off, "does nothing"),
            (ModelGuardCommand::Warn, Mode::Warn, "says what it costs"),
        ] {
            let text = run_command(&sb, &command);
            assert_eq!(load_config(&sb.env).modes.model_guard, mode, "{command:?}");
            assert!(text.contains("[modes] model_guard"), "{text}");
            assert!(text.contains(words), "{words} in {text}");
        }
    }

    #[test]
    fn the_mode_is_written_with_the_names_the_user_typed() {
        let sb = configured();
        run_command(&sb, &ModelGuardCommand::On);
        let file = std::fs::read_to_string(sb.env.data.config_path()).unwrap();
        assert!(file.contains("model_guard = \"on\""), "{file}");
    }

    #[test]
    fn limit_sets_the_context_size_and_keeps_the_other_keys() {
        let sb = configured();
        run_command(&sb, &ModelGuardCommand::Off);
        let text = run_command(&sb, &ModelGuardCommand::Limit { tokens: 1_000 });
        let config = load_config(&sb.env);
        assert_eq!(config.model_guard.min_context_tokens, 1_000);
        assert_eq!(config.modes.model_guard, Mode::Off);
        assert!(text.contains("1,000") || text.contains("1000"), "{text}");
    }

    #[test]
    fn a_limit_too_large_for_the_config_file_is_refused() {
        let sb = configured();
        let (mut io, _) = scripted(false, "");
        let err = run(
            &sb.env,
            &mut io,
            &ModelGuardCommand::Limit { tokens: u64::MAX },
        )
        .unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
        assert_eq!(load_config(&sb.env).model_guard.min_context_tokens, 100_000);
    }

    #[test]
    fn status_shows_the_mode_and_the_limit() {
        let sb = configured();
        let text = run_command(&sb, &ModelGuardCommand::Status);
        assert!(text.contains("Mode:   warn"), "{text}");
        assert!(
            text.contains("100000") || text.contains("100,000"),
            "{text}"
        );
        run_command(&sb, &ModelGuardCommand::On);
        run_command(&sb, &ModelGuardCommand::Limit { tokens: 5_000 });
        let text = run_command(&sb, &ModelGuardCommand::Status);
        assert!(text.contains("Mode:   on"), "{text}");
        assert!(text.contains("5000") || text.contains("5,000"), "{text}");
    }

    #[test]
    fn a_config_file_that_is_not_toml_is_an_error_and_is_left_alone() {
        let sb = configured();
        let path = sb.env.data.config_path();
        std::fs::write(&path, "[modes\n").unwrap();
        let (mut io, _) = scripted(false, "");
        let err = run(&sb.env, &mut io, &ModelGuardCommand::On).unwrap_err();
        assert!(err.to_string().contains("not valid TOML"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[modes\n");
    }
}
