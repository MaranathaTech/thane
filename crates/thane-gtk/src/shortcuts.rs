use gtk4::prelude::*;
use gtk4::{ShortcutController, ShortcutScope};

use thane_core::config::Config;
use thane_core::keybinding::{KeyAction, Keybinding, default_keybindings, merge_keybindings};

/// Callback type for keyboard shortcut actions.
pub type ShortcutHandler = Box<dyn Fn(KeyAction) + 'static>;

/// Set up keyboard shortcuts on a GTK widget, merging defaults with config overrides.
pub fn setup_shortcuts_with_config(
    widget: &impl IsA<gtk4::Widget>,
    config: &Config,
    handler: ShortcutHandler,
) {
    let user_bindings = config.keybindings();
    let bindings = merge_keybindings(default_keybindings(), &user_bindings);
    setup_shortcuts_from_bindings(widget, &bindings, handler);
}

/// Set up keyboard shortcuts on a GTK widget using default bindings.
pub fn setup_shortcuts(widget: &impl IsA<gtk4::Widget>, handler: ShortcutHandler) {
    setup_shortcuts_from_bindings(widget, &default_keybindings(), handler);
}

fn setup_shortcuts_from_bindings(
    widget: &impl IsA<gtk4::Widget>,
    bindings: &[Keybinding],
    handler: ShortcutHandler,
) {
    let controller = ShortcutController::new();
    controller.set_scope(ShortcutScope::Global);

    let handler = std::rc::Rc::new(handler);

    for binding in bindings {
        let trigger = binding_to_trigger(binding);
        if let Some(trigger) = trigger {
            let action = binding.action.clone();
            let handler = handler.clone();

            let shortcut_action =
                gtk4::CallbackAction::new(move |_widget, _args| {
                    handler(action.clone());
                    glib::Propagation::Stop
                });

            let shortcut = gtk4::Shortcut::new(Some(trigger), Some(shortcut_action));
            controller.add_shortcut(shortcut);
        }
    }

    widget.add_controller(controller);
}

/// Convert a Keybinding to a GTK ShortcutTrigger string.
fn binding_to_trigger(binding: &Keybinding) -> Option<gtk4::ShortcutTrigger> {
    let mut parts = Vec::new();

    if binding.modifiers.ctrl {
        parts.push("<Ctrl>");
    }
    if binding.modifiers.alt {
        parts.push("<Alt>");
    }
    if binding.modifiers.shift {
        parts.push("<Shift>");
    }
    if binding.modifiers.super_key {
        parts.push("<Super>");
    }

    let trigger_str = format!("{}{}", parts.join(""), binding.key);
    gtk4::ShortcutTrigger::parse_string(&trigger_str)
}
