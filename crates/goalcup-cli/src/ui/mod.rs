//! Terminal presentation only: assets, colours, rendering and menu input.
//!
//! This layer receives labels and returns a selection. It does not discover connectors,
//! save settings, open URLs, sign in, or start a harness. Those actions belong to the caller.
//! Changing a wordmark or replacing this renderer must not change a capture operation.

mod assets;
mod menu;
mod terminal;
pub mod theme;

pub use menu::{Choice, Menu, render};
pub use terminal::{interactive, select};

/// Setup copy and presentation stay here; the caller retains IDs and applies the choice.
/// The final row returns `harnesses.len()`, letting the caller handle its guide action.
pub fn choose_harness(harnesses: &[(&str, bool)]) -> std::io::Result<Option<usize>> {
    let mut choices: Vec<_> = harnesses
        .iter()
        .map(|(label, installed)| Choice {
            label: (*label).to_owned(),
            hint: (!installed).then(|| "not installed".into()),
        })
        .collect();
    choices.push(Choice {
        label: "Add or request a harness".into(),
        hint: None,
    });
    select(&Menu {
        title: "Choose your harness",
        subtitle: "One harness for your hack.",
        choices: &choices,
    })
}
