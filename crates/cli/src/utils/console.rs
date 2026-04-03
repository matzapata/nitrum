use anyhow::{Context, Result};
use dialoguer::theme::ColorfulTheme;
use indicatif::{ProgressBar, ProgressStyle};
use std::future::Future;
use std::path::Path;
use std::time::Duration;

/// Sets up a spinner with the given message and returns it.
///
/// # Panics
///
/// Panics if the underlying `indicatif::ProgressStyle` template is invalid.
#[must_use]
pub fn style_spinner(spinner: ProgressBar, message: &str) -> ProgressBar {
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.blue} {msg}")
            .unwrap()
            .tick_strings(&["▓", "▒", "░"]),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(100));
    spinner.tick();

    spinner
}

/// Runs `fut` with a spinner and returns the result.
///
/// # Errors
///
/// Propagates any error returned by `fut` unchanged.
pub async fn with_spinner<T, E, Fut>(message: &str, success_message: &str, fut: Fut) -> Result<T, E>
where
    Fut: Future<Output = Result<T, E>>,
{
    let spinner = style_spinner(ProgressBar::new_spinner(), message);
    match fut.await {
        Ok(value) => {
            spinner.finish_with_message(success_message.to_string());
            Ok(value)
        }
        Err(err) => {
            spinner.finish_and_clear();
            Err(err)
        }
    }
}

/// Writes `value` as pretty-printed JSON to `path` (overwrites if present).
///
/// # Errors
///
/// Returns an error when the file cannot be created or written.
pub fn write_json_value_pretty(path: impl AsRef<Path>, value: &serde_json::Value) -> Result<()> {
    let path = path.as_ref();
    let out_file =
        std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    serde_json::to_writer_pretty(out_file, value)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Prompts the user for input and returns the result.
///
/// # Panics
///
/// Panics if the underlying dialoguer input interaction fails.
#[must_use]
pub fn prompt(message: &str) -> String {
    dialoguer::Input::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .interact_text()
        .expect("Failed to prompt")
}

/// Prompts the user for confirmation and returns the result.
///
/// # Panics
///
/// Panics if the underlying dialoguer confirmation interaction fails.
#[must_use]
pub fn confirm(message: &str) -> bool {
    dialoguer::Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .interact()
        .expect("Failed to confirm")
}
