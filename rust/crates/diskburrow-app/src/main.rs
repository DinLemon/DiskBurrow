#![windows_subsystem = "windows"]
mod contract;
mod helper;
mod helper_args;
mod input;
mod locale;
mod map_view;
mod operation;
mod platform;
mod recommendations;
mod runtime;
mod ui;
mod verify;

use anyhow::{Result, ensure};
use gpui_kit::{
    AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{path::PathBuf, sync::mpsc};

fn main() {
    if let Err(error) = run() {
        eprintln!("DiskBurrow: {error:#}");
        if !std::env::args().skip(1).any(|arg| {
            matches!(
                arg.as_str(),
                "--mft-helper" | "--data-dir" | "--verify-runtime" | "--help"
            )
        }) {
            platform::startup_error(&format!("DiskBurrow could not start.\n\n{error:#}"));
        }
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
    let mut data = None;
    let mut root = None;
    let mut background = false;
    let mut diagnostic = false;
    let mut verification = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                i += 1;
                data = Some(PathBuf::from(
                    args.get(i)
                        .ok_or_else(|| anyhow::anyhow!("Missing data directory"))?,
                ));
                diagnostic = true;
            }
            "--scan-root" => {
                i += 1;
                root = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow::anyhow!("Missing scan root"))?
                        .clone(),
                );
            }
            "--background" => background = true,
            "--verify-runtime" => {
                i += 1;
                verification =
                    Some(PathBuf::from(args.get(i).ok_or_else(|| {
                        anyhow::anyhow!("Missing verification report path")
                    })?));
            }
            "--help" => {
                println!(
                    "DiskBurrow [--background] [--data-dir ABSOLUTE_DIRECTORY] [--scan-root LOCAL_DIRECTORY] [--verify-runtime NEW_ABSOLUTE_REPORT]"
                );
                return Ok(());
            }
            _ => anyhow::bail!("Unknown argument: {}", args[i]),
        }
        i += 1;
    }
    let data = data.map_or_else(platform::app_data_directory, Ok)?;
    ensure!(data.is_absolute(), "An absolute data directory is required");
    if let Some(report) = verification {
        ensure!(
            diagnostic,
            "Runtime verification requires an explicit --data-dir"
        );
        return verify::run(
            data,
            root.ok_or_else(|| {
                anyhow::anyhow!("Runtime verification requires an explicit --scan-root")
            })?,
            &report,
        );
    }
    let mut runtime = runtime::Runtime::new(data)?;
    if let Some(root) = root {
        runtime.command(contract::Command::SetRoot(root));
    }
    if !diagnostic {
        let (tx, rx) = mpsc::channel();
        let bridge = platform::PlatformBridge::start(tx)?;
        if !bridge.is_primary() {
            return Ok(());
        }
        runtime.attach_bridge(bridge, rx);
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
                        window.on_window_should_close(cx, move |_, cx| {
                            if diagnostic {
                                let _ = weak.update(cx, |app, cx| {
                                    app.runtime.command(contract::Command::Exit);
                                    cx.notify();
                                });
                            } else {
                                cx.hide();
                            }
                            false
                        });
                        entity
                    },
                )
                .expect("Create the DiskBurrow window");
            if background {
                cx.hide();
            } else {
                cx.activate(true);
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
