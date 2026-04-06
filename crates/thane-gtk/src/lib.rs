pub mod agent;
pub mod application;
pub mod browser;
pub mod css;
pub mod fonts;
pub mod rpc_handler;
pub mod setup;
pub mod shortcuts;
pub mod sidebar;
pub mod split;
pub mod terminal;
pub mod workspace;
pub mod window;

/// Entry point for the GTK application.
pub fn run() {
    application::run_app();
}
