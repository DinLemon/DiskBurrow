#![windows_subsystem = "windows"]
mod appearance;
mod cli;
mod contract;
mod git_inspection;
mod helper;
mod helper_args;
mod input;
mod launch_ipc;
mod locale;
mod map_view;
mod mark_selection;
mod operation;
mod platform;
mod recommendations;
mod runtime;
mod scan_cache;
mod selection_export;
mod ui;
mod verify;

use anyhow::{Result, ensure};
use gpui_kit::{
    AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::mpsc;

fn main() {
    if let Err(error) = run() {
        let console = platform::ParentConsole::attach();
        eprintln!("DiskBurrow: {error:#}");
        if !std::env::args().skip(1).any(|arg| {
            matches!(
                arg.as_str(),
                "--mft-helper" | "--data-dir" | "--verify-runtime" | "--help" | "-h"
            )
        }) {
            platform::startup_error(&format!("DiskBurrow could not start.\n\n{error:#}"));
        }
        drop(console);
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(helper) = helper::parse(&args)? {
        return helper::run(helper);
    }
    ensure!(
        !helper::is_elevated()?,
        "Start DiskBurrow as an ordinary user; only the separate NTFS reader requests administrator approval"
    );
    let mut options = cli::parse(&args)?;
    if options.help {
        platform::print_usage(cli::USAGE)?;
        return Ok(());
    }
    platform::validate_verification_launch(&args, &options)?;
    options.resolve(&std::env::current_dir()?, &platform::system_root())?;
    let diagnostic = options.data_dir.is_some();
    let background = options.background;
    let data = options
        .data_dir
        .map_or_else(platform::app_data_directory, Ok)?;
    ensure!(data.is_absolute(), "An absolute data directory is required");
    if let Some(report) = options.verification {
        return verify::run(data, options.launch.root.unwrap(), &report);
    }
    let bridge = if !diagnostic {
        let (tx, rx) = mpsc::channel();
        let payload = options
            .launch
            .is_explicit()
            .then(|| serde_json::to_string(&options.launch))
            .transpose()?;
        let bridge = if payload.is_some() {
            platform::PlatformBridge::start_with_launch(tx, payload.as_deref())?
        } else {
            platform::PlatformBridge::start(tx)?
        };
        if !bridge.is_primary() {
            return Ok(());
        }
        Some((bridge, rx))
    } else {
        None
    };
    let mut runtime = runtime::Runtime::new(data)?;
    if let Some((bridge, rx)) = bridge {
        runtime.attach_bridge(bridge, rx);
    }
    if options.launch.is_explicit() {
        runtime.apply_launch(options.launch)?;
    }
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            let handle = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(120.), px(90.)),
                            size(px(1140.), px(780.)),
                        ))),
                        window_min_size: Some(size(px(880.), px(600.))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("DiskBurrow · Rust preview".into()),
                            ..Default::default()
                        }),
                        app_id: Some("DiskBurrow".into()),
                        show: !background,
                        ..Default::default()
                    },
                    move |window, cx| {
                        let entity = cx.new(|cx| ui::App::new(runtime, window, cx));
                        let weak = entity.downgrade();
                        window.on_window_should_close(cx, move |window, cx| {
                            if diagnostic {
                                let _ = weak.update(cx, |app, cx| {
                                    app.runtime.command(contract::Command::Exit);
                                    cx.notify();
                                });
                            } else {
                                if let Ok(handle) = window.window_handle()
                                    && let RawWindowHandle::Win32(handle) = handle.as_raw()
                                {
                                    platform::hide_own_window(handle.hwnd.get());
                                }
                            }
                            false
                        });
                        entity
                    },
                )
                .expect("Create the DiskBurrow window");
            if !background {
                let _ = handle.update(cx, |_, window, _| {
                    if let Ok(handle) = window.window_handle()
                        && let RawWindowHandle::Win32(handle) = handle.as_raw()
                    {
                        platform::restore_own_window(handle.hwnd.get());
                    }
                    window.activate_window();
                });
            }
            let _ = handle;
        });
    Ok(())
}
