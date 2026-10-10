#![cfg(windows)]
// Exercise app Windows integration policies without GPUI, registry mutation or a real tray.
#[allow(dead_code)]
#[path = "../../diskburrow-app/src/platform.rs"]
mod platform;

#[allow(dead_code)]
#[path = "../../diskburrow-app/src/cli.rs"]
mod cli;
#[allow(dead_code)]
#[path = "../../diskburrow-app/src/launch_ipc.rs"]
mod launch_ipc;
