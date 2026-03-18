use dialoguer::theme::ColorfulTheme;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

pub fn style_spinner(spinner: ProgressBar, message: &str) -> ProgressBar {
    spinner.set_style(ProgressStyle::with_template("{spinner:.blue} {msg}").unwrap());
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(100));
    spinner.tick();

    spinner
}

pub fn prompt(message: &str) -> String {
    dialoguer::Input::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .interact_text()
        .expect("Failed to prompt")
}

pub fn confirm(message: &str) -> bool {
    dialoguer::Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .interact()
        .expect("Failed to confirm")
}
