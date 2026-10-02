//! Command dispatch. Each area module declares its commands with
//! `area_commands![...]`, which generates `COMMANDS` and `handler()`.
//! To add a command, write it in its area file and add it to that file's
//! `area_commands!` list; this file doesn't change.

use tauri::ipc::Invoke;
use tauri::Wry;

/// Declares an area's command list: `area_commands![cmd_a, cmd_b];`
macro_rules! area_commands {
    ($($cmd:ident),* $(,)?) => {
        pub const COMMANDS: &[&str] = &[$(stringify!($cmd)),*];
        pub fn handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$($cmd),*]
        }
    };
}
pub(crate) use area_commands;

pub mod app;
pub mod catalog;
pub mod describe;
pub mod downloads;
pub mod generate;
pub mod library;
pub mod models;

pub fn dispatch(invoke: Invoke<Wry>) -> bool {
    let cmd = invoke.message.command().to_string();
    let c = cmd.as_str();
    if app::COMMANDS.contains(&c) {
        return (app::handler())(invoke);
    }
    if catalog::COMMANDS.contains(&c) {
        return (catalog::handler())(invoke);
    }
    if describe::COMMANDS.contains(&c) {
        return (describe::handler())(invoke);
    }
    if downloads::COMMANDS.contains(&c) {
        return (downloads::handler())(invoke);
    }
    if generate::COMMANDS.contains(&c) {
        return (generate::handler())(invoke);
    }
    if library::COMMANDS.contains(&c) {
        return (library::handler())(invoke);
    }
    if models::COMMANDS.contains(&c) {
        return (models::handler())(invoke);
    }
    false
}
