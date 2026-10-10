//! Programmatic native GPUI frames and input dispatch; not manual desktop acceptance.
use super::*;
use diskburrow_services::SettingsStore;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, point, size};

fn idle(runtime: &mut Runtime) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while runtime.view().busy {
        runtime.poll();
        assert!(
            std::time::Instant::now() < deadline,
            "owned fixture worker timeout"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn scroll_to(window: &mut Window, container: &str, target: &str, cx: &mut gpui_kit::App) {
    window.scroll(
        container.to_owned(),
        gpui_kit::ScrollDelta::Pixels(point(px(0.), px(10000.))),
        cx,
    );
    for _ in 0..32 {
        let clip = window.find(container.to_owned()).bounds();
        let item = window.find(target.to_owned());
        if item.visible()
            && item.bounds().top() >= clip.top()
            && item.bounds().bottom() <= clip.bottom()
        {
            return;
        }
        window.scroll(
            container.to_owned(),
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-48.))),
            cx,
        );
    }
    panic!("{target} never became readable inside {container}");
}

fn inside_minimum_window(window: &Window, id: &str) {
    let item = window.find(id.to_owned());
    let bounds = item.bounds();
    assert!(item.visible(), "{id} must be visible: {bounds:?}");
    assert!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "{id} has no area"
    );
    assert!(
        bounds.left() >= px(0.) && bounds.right() <= px(880.),
        "{id} exceeds window width: {bounds:?}"
    );
    assert!(
        bounds.top() >= px(0.) && bounds.bottom() <= px(600.),
        "{id} exceeds window height: {bounds:?}"
    );
}

fn owned_fixture() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("diskburrow-ui-port-")
        .tempdir()
        .unwrap()
}

#[gpui_kit::test]
fn port_review_capped_sidebar_drag_uses_rendered_width(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    cx.update(gpui_omarchy::init);
    for scale in [75, 100, 150] {
        let mut runtime = scanned_runtime(fixture.path(), "en", scale);
        runtime.command(Command::Setting(Setting::SidebarWidth, "420".into()));
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            App::new(runtime, window, cx)
        });
        handle
            .update(cx, |app, _, cx| {
                app.page = Page::Map;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let before = window.find("map-sidebar").bounds().size.width.as_f32();
            let canvas = window.find("map-viewport").bounds().size.width.as_f32();
            assert!(before < 400., "fixture must exercise the 45% cap");
            let from = window.find("map-sidebar-divider").bounds().center();
            window.drag(from, point(from.x + px(20.), from.y), cx);
            window.render_frame(cx);
            let after = window.find("map-sidebar").bounds().size.width.as_f32();
            let grown = window.find("map-viewport").bounds().size.width.as_f32();
            assert!(
                (before - after - 20.).abs() <= 1.,
                "scale {scale}: {before} -> {after}"
            );
            assert!(
                (grown - canvas - 20.).abs() <= 1.,
                "scale {scale}: canvas {canvas} -> {grown}"
            );
            window.double_click("map-sidebar-divider", cx);
            assert_eq!(window.find("map-sidebar").bounds().size.width, px(225.));
            let from = window.find("map-sidebar-divider").bounds().center();
            window.drag(from, point(from.x + px(20.), from.y), cx);
            window.render_frame(cx);
            assert!((window.find("map-sidebar").bounds().size.width.as_f32() - 205.).abs() <= 1.);
        })
        .unwrap();
        handle
            .update(cx, |app, _, cx| {
                app.dispatch(Command::Setting(Setting::SidebarWidth, "420".into()), cx)
            })
            .unwrap();
        cx.simulate_window_resize(handle.into(), size(px(1280.), px(700.)));
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("map-sidebar").bounds().size.width, px(420.));
            let from = window.find("map-sidebar-divider").bounds().center();
            window.drag(from, point(from.x + px(20.), from.y), cx);
            window.render_frame(cx);
            assert_eq!(window.find("map-sidebar").bounds().size.width, px(400.));
        })
        .unwrap();
        cx.simulate_window_resize(handle.into(), size(px(880.), px(600.)));
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let before = window.find("map-sidebar").bounds().size.width.as_f32();
            assert!(before < 400.);
            let from = window.find("map-sidebar-divider").bounds().center();
            window.drag(from, point(from.x + px(20.), from.y), cx);
            window.render_frame(cx);
            assert!(
                (before - window.find("map-sidebar").bounds().size.width.as_f32() - 20.).abs()
                    <= 1.
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn port_review_widen_publication_rejects_old_hover_and_row_indices(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    let parent = fixture.path().join("tree");
    let root = parent.join("scan");
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("kept.txt");
    std::fs::write(&file, b"kept").unwrap();
    cx.update(gpui_omarchy::init);
    for action in [
        "space",
        "enter",
        "row-mark",
        "row-navigate",
        "row-focus",
        "cancel",
    ] {
        let mut runtime = Runtime::new(fixture.path().join(format!("data-{action}"))).unwrap();
        idle(&mut runtime);
        runtime.command(Command::SetRoot(root.to_string_lossy().into_owned()));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        let old_index = runtime
            .map_tiles(600., 300.)
            .iter()
            .find(|tile| tile.name == "kept.txt")
            .unwrap()
            .index;
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            App::new(runtime, window, cx)
        });
        handle
            .update(cx, |app, _, cx| {
                app.page = Page::Map;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        handle
            .update(cx, |app, _, cx| {
                app.dispatch(Command::MapWiden(parent.to_string_lossy().into_owned()), cx);
                assert!(app.runtime.view().busy);
                // Retained-map interaction can also change zoom while the worker is pending.
                app.transform.scale = 2.;
                app.runtime.command(Command::MapZoom(2.));
            })
            .unwrap();
        // Dispatch a real old-map hover while the previous completed map is retained.
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let bounds = window.find("map-viewport").bounds();
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: bounds.center(),
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
        })
        .unwrap();
        handle
            .update(cx, |app, window, cx| {
                assert_eq!(app.active_tile(), Some(old_index));
                if action == "cancel" {
                    app.dispatch(Command::Cancel, cx);
                }
                // No frame is allowed between index publication and the queued old-frame input.
                idle(&mut app.runtime);
                if action == "cancel" {
                    assert_eq!(app.runtime.map_path(old_index).as_deref(), file.to_str());
                    assert_eq!(app.transform.scale, 2.);
                    assert_eq!(app.runtime.view().map_zoom, 2.);
                    app.dispatch(Command::MapMark(old_index), cx);
                    assert_eq!(app.runtime.view().selected_count, 1);
                    return;
                }
                assert_eq!(
                    app.runtime.map_path(old_index).as_deref(),
                    root.to_str(),
                    "fixture must change index meaning"
                );
                match action {
                    "space" | "enter" => app.map_key(
                        &KeyDownEvent {
                            keystroke: gpui_kit::Keystroke::parse(action).unwrap(),
                            is_held: false,
                            prefer_character_input: false,
                        },
                        window,
                        cx,
                    ),
                    "row-mark" => app.dispatch(Command::MapMark(old_index), cx),
                    "row-navigate" => app.dispatch(Command::MapNavigate(old_index), cx),
                    _ => app.dispatch(Command::MapFocus(old_index), cx),
                }
                assert_eq!(
                    app.runtime.view().selected_count,
                    0,
                    "{action} marked an entry from a different index"
                );
                assert_eq!(app.active_tile(), None, "{action} retained a stale target");
                assert!(
                    app.map_tiles.borrow().is_empty(),
                    "{action} retained stale hit rectangles"
                );
                assert_eq!(app.transform.scale, 1.);
                assert_eq!(
                    app.runtime.view().map_zoom,
                    1.,
                    "new index zoom label must match its transform"
                );
            })
            .unwrap();
        if action == "cancel" {
            continue;
        }
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        handle
            .update(cx, |app, _, cx| {
                let fresh_index = app
                    .map_tiles
                    .borrow()
                    .iter()
                    .find(|tile| tile.name == "kept.txt")
                    .unwrap()
                    .index;
                assert_ne!(fresh_index, old_index);
                app.dispatch(Command::MapMark(fresh_index), cx);
                assert_eq!(
                    app.runtime.view().selected_count,
                    1,
                    "fresh frame remains interactive"
                );
            })
            .unwrap();
    }
    assert_eq!(std::fs::read(file).unwrap(), b"kept");
}

fn scanned_runtime(fixture: &std::path::Path, language: &str, scale: u16) -> Runtime {
    let root = fixture.join("scan");
    for name in ["alpha", "beta"] {
        let directory = root.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("first.bin"), vec![1_u8; 16384]).unwrap();
        std::fs::write(directory.join("second.bin"), vec![2_u8; 8192]).unwrap();
    }
    let mut runtime = Runtime::new(fixture.join(format!("data-{language}-{scale}"))).unwrap();
    idle(&mut runtime);
    runtime.command(Command::Setting(Setting::Language, language.into()));
    runtime.command(Command::Setting(Setting::Theme, "dark".into()));
    runtime.command(Command::Setting(Setting::UiScale, scale.to_string()));
    runtime.command(Command::SetRoot(root.to_string_lossy().into_owned()));
    runtime.command(Command::Scan(false));
    idle(&mut runtime);
    assert!(runtime.view().error.is_none(), "{:?}", runtime.view().error);
    assert!(runtime.view().map_has_data);
    runtime
}

#[gpui_kit::test]
fn port_six_pages_keep_primary_controls_and_bounded_tables_at_150_percent(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    cx.update(gpui_omarchy::init);
    for language in ["ru", "en"] {
        let runtime = scanned_runtime(fixture.path(), language, 150);
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            App::new(runtime, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            for nav in [
                "nav-overview",
                "nav-largest",
                "nav-map",
                "nav-history",
                "nav-cleanup",
                "nav-settings",
            ] {
                window.click(nav, cx);
                inside_minimum_window(window, "scan-root");
                assert!(
                    window.find("scan-root").bounds().size.width >= px(100.),
                    "{language}/{nav}"
                );
                inside_minimum_window(window, "scan-normal");
                inside_minimum_window(window, "status-bar");
                let (controls, tables): (&[&str], &[&str]) = match nav {
                    "nav-overview" => (&[], &["overview-issues"]),
                    "nav-largest" => (
                        &["largest-folders", "largest-files", "manual-review"],
                        &["largest-table"],
                    ),
                    "nav-map" => (&["map-help", "map-search", "map-viewport"], &[]),
                    "nav-history" => (
                        &["history-scans", "history-changes"],
                        &["history-scans-table"],
                    ),
                    "nav-cleanup" => (
                        &[
                            "analyze-cleanup",
                            "cleanup-filter",
                            "cleanup-tab-candidates",
                        ],
                        &["cleanup-candidates"],
                    ),
                    _ => (&["settings-save"], &[]),
                };
                for id in controls {
                    inside_minimum_window(window, id);
                }
                if let Some(search) = match nav {
                    "nav-map" => Some("map-search"),
                    "nav-cleanup" => Some("cleanup-filter"),
                    _ => None,
                } {
                    assert!(
                        window.find(search.to_owned()).bounds().size.width >= px(100.),
                        "{language}/{nav} search input is too narrow"
                    );
                }
                for table in tables {
                    let container = format!("{table}-container");
                    let header = format!("{table}-header");
                    inside_minimum_window(window, &container);
                    inside_minimum_window(window, &header);
                    assert!(
                        window.find(container).bounds().size.width >= px(180.),
                        "{language}/{nav}"
                    );
                    assert!(
                        window.find(header).bounds().size.height >= px(20.),
                        "{language}/{nav}"
                    );
                }
                if nav == "nav-settings" {
                    scroll_to(window, "settings-scroll", "settings-low", cx);
                    inside_minimum_window(window, "settings-low");
                    assert!(window.find("settings-low").bounds().size.width > px(100.));
                    scroll_to(window, "settings-scroll", "settings-custom-temp", cx);
                    inside_minimum_window(window, "settings-custom-temp");
                    inside_minimum_window(window, "settings-save");
                }
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn port_owned_scan_keyboard_siblings_search_escape_and_sidebar_drag_use_native_dispatch(
    cx: &mut TestAppContext,
) {
    let fixture = owned_fixture();
    let runtime = scanned_runtime(fixture.path(), "en", 100);
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-map", cx);
    })
    .unwrap();
    let (alpha, offset) = handle
        .update(cx, |app, _, _| {
            let tiles = app.map_tiles.borrow();
            let tile = tiles
                .iter()
                .find(|tile| tile.name == "alpha" && tile.directory)
                .unwrap();
            (tile.index, point(px(tile.x + 1.), px(tile.y + 1.)))
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.click_at("map-viewport", offset, cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.active_tile(), Some(alpha));
            assert!(!app.keyboard_target);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("tab", cx))
        .unwrap();
    let next = handle
        .update(cx, |app, _, _| {
            assert!(app.keyboard_target);
            let next = app.active_tile().unwrap();
            assert_ne!(next, alpha);
            assert!(
                app.runtime
                    .siblings(alpha)
                    .iter()
                    .any(|row| row.key == next.to_string())
            );
            assert_eq!(
                app.runtime.view().focused_path,
                app.runtime.map_path(next).unwrap()
            );
            next
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("enter", cx))
        .unwrap();
    let sibling_button = handle
        .update(cx, |app, _, _| {
            assert_eq!(
                app.runtime.view().map_path,
                app.runtime.map_path(next).unwrap()
            );
            format!(
                "map-siblings-{}",
                app.runtime.view().map_breadcrumbs.len() - 1
            )
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(sibling_button, cx);
        inside_minimum_window(window, "siblings-close");
        assert!(window.find("sibling-0").visible());
        assert!(window.find("sibling-1").visible());
        window.press("escape", cx);
        assert!(window.try_find("siblings-dialog").is_none());
        window.click("map-viewport", cx);
        window.press("f", cx);
        window.input("ahig[]", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, cx| {
            assert_eq!(app.search.read(cx).value(), "ahig[]");
            assert_eq!(app.runtime.view().map_query, "ahig[]");
            assert_eq!(app.runtime.view().settings.map_depth, 3);
            assert!(app.runtime.view().settings.show_hidden);
            assert_eq!(app.runtime.view().map_color, 0);
            assert!(!app.runtime.view().map_global);
            assert!(!app.runtime.view().map_isolate);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("escape", cx);
        window.click("map-viewport", cx);
        window.press("f1", cx);
        assert!(window.find("keys-dialog").visible());
        window.press("escape", cx);
        assert!(window.try_find("keys-dialog").is_none());
        let from = window.find("map-sidebar-divider").bounds().center();
        window.drag(from, point(from.x - px(40.), from.y), cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, cx| {
            assert_eq!(app.search.read(cx).value(), "");
            assert_eq!(app.runtime.view().map_query, "");
            assert!(!app.sidebar_drag);
            assert!((226..=420).contains(&app.runtime.view().settings.sidebar_width));
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.double_click("map-sidebar-divider", cx);
        inside_minimum_window(window, "map-viewport");
        assert!(window.find("map-viewport").bounds().size.width >= px(180.));
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().settings.sidebar_width, 225);
            assert_eq!(
                app.runtime.view().map_path,
                app.runtime.map_path(next).unwrap()
            );
        })
        .unwrap();
}

#[gpui_kit::test]
fn port_minimum_frames_keep_map_help_and_display_settings_usable_in_all_variants(
    cx: &mut TestAppContext,
) {
    let fixture = owned_fixture();
    cx.update(gpui_omarchy::init);
    for language in ["ru", "en"] {
        for theme in ["light", "dark", "system"] {
            for scale in [75, 100, 150] {
                let variant = format!("{language}-{theme}-{scale}");
                let directory = fixture.path().join(&variant);
                let mut runtime = Runtime::new(directory.clone()).unwrap();
                idle(&mut runtime);
                for (setting, value) in [
                    (Setting::Language, language.to_owned()),
                    (Setting::Theme, theme.to_owned()),
                    (Setting::UiScale, scale.to_string()),
                    (Setting::MapDepth, "6".into()),
                    (Setting::ShowHidden, "false".into()),
                    (Setting::SidebarWidth, "420".into()),
                ] {
                    runtime.command(Command::Setting(setting, value));
                }
                let mut entity = None;
                let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
                    entity = Some(cx.entity());
                    App::new(runtime, window, cx)
                });
                let app = entity.unwrap();
                cx.update_window(handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    window.click("nav-map", cx);
                    for id in [
                        "map-help",
                        "map-age",
                        "map-hidden",
                        "map-depth-less",
                        "map-depth-more",
                        "map-search",
                    ] {
                        inside_minimum_window(window, id);
                    }
                    assert_eq!(
                        window.find("map-age").label(),
                        Some(crate::locale::text(language, "Age.Mode").as_str()),
                        "{variant}"
                    );
                    let viewport = window.find("map-viewport");
                    inside_minimum_window(window, "map-viewport");
                    assert!(viewport.bounds().size.width >= px(180.), "{variant}");
                    assert!(viewport.bounds().size.height >= px(180.), "{variant}");
                    inside_minimum_window(window, "map-sidebar");
                    assert!(
                        window.find("map-sidebar").bounds().size.width >= px(180.),
                        "{variant}"
                    );
                    window.click("map-help", cx);
                    inside_minimum_window(window, "keys-close");
                    assert!(window.find("keys-dialog").visible());
                    window.click("keys-close", cx);
                    assert!(window.try_find("keys-dialog").is_none());
                    window.click("map-hidden", cx);
                    window.click("map-depth-less", cx);
                    scroll_to(window, "map-sidebar", "sidebar-reset", cx);
                    window.click("sidebar-reset", cx);
                    window.click("nav-settings", cx);
                    for (id, key) in [
                        ("theme-light", "Theme.Light"),
                        ("theme-dark", "Theme.Dark"),
                        ("theme-system", "Settings.Theme.System"),
                    ] {
                        scroll_to(window, "settings-scroll", id, cx);
                        inside_minimum_window(window, id);
                        assert_eq!(
                            window.find(id).label(),
                            Some(crate::locale::text(language, key).as_str()),
                            "{variant}"
                        );
                    }
                    for percent in [75, 90, 100, 110, 125, 150] {
                        let id = format!("ui-scale-{percent}");
                        scroll_to(window, "settings-scroll", &id, cx);
                        inside_minimum_window(window, &id);
                        assert_eq!(
                            window.find(id).label(),
                            Some(format!("{percent}%").as_str())
                        );
                    }
                    scroll_to(window, "settings-scroll", "settings-custom-temp", cx);
                    inside_minimum_window(window, "settings-custom-temp");
                    inside_minimum_window(window, "settings-save");
                    assert_eq!(
                        window.find("settings-save").label(),
                        Some(crate::locale::text(language, "Action.Save").as_str())
                    );
                    window.click("settings-save", cx);
                })
                .unwrap();
                app.update(cx, |app, _| {
                    idle(&mut app.runtime);
                    assert!(
                        app.runtime.view().error.is_none(),
                        "{variant}: {:?}",
                        app.runtime.view().error
                    );
                });
                let saved = SettingsStore::new(directory).load();
                assert_eq!(saved.language, language, "{variant}");
                assert_eq!(saved.theme, theme, "{variant}");
                assert_eq!(saved.ui_scale_percent, scale, "{variant}");
                assert_eq!(saved.map_depth, 5, "{variant}");
                assert!(saved.show_hidden, "{variant}");
                assert_eq!(saved.sidebar_width, 225, "{variant}");
            }
        }
    }
}

#[gpui_kit::test]
fn port_control_interface_zoom_leaves_map_transform_and_zoom_unchanged(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
    idle(&mut runtime);
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    handle
        .update(cx, |app, _, cx| {
            app.page = Page::Map;
            app.transform.scale = 2.;
            app.runtime.command(Command::MapZoom(2.));
            cx.notify();
        })
        .unwrap();
    for (key, percent) in [
        ("ctrl-=", 110),
        ("ctrl--", 100),
        ("ctrl--", 90),
        ("ctrl-0", 100),
    ] {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("map-viewport", cx);
            window.press(key, cx);
        })
        .unwrap();
        handle
            .update(cx, |app, _, _| {
                assert_eq!(
                    app.runtime.view().settings.ui_scale_percent,
                    percent,
                    "{key}"
                );
                assert_eq!(app.runtime.view().map_zoom, 2., "{key}");
                assert_eq!(app.transform.scale, 2., "{key}");
            })
            .unwrap();
    }
}

#[gpui_kit::test]
fn port_native_editors_keep_typing_and_do_not_dispatch_map_letter_shortcuts(
    cx: &mut TestAppContext,
) {
    let fixture = owned_fixture();
    let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
    idle(&mut runtime);
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-map", cx);
        window.click("map-viewport", cx);
        window.click("map-search", cx);
        window.input("ahigc[]xhjkltdvpoqrus?", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, cx| {
            assert_eq!(app.search.read(cx).value(), "ahigc[]xhjkltdvpoqrus");
            assert_eq!(app.runtime.view().map_query, "ahigc[]xhjkltdvpoqrus");
            assert_eq!(app.runtime.view().map_color, 0);
            assert_eq!(app.runtime.view().settings.map_depth, 3);
            assert!(app.runtime.view().settings.show_hidden);
            assert!(!app.runtime.view().map_isolate);
            assert!(!app.runtime.view().map_global);
            assert!(!app.keys_open);
            assert!(!app.volumes_open);
            assert!(!app.runtime.should_exit());
        })
        .unwrap();
    let root_value = fixture.path().join("ahigc").to_string_lossy().into_owned();
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("scan-root", cx);
        window.press("ctrl-a", cx);
        window.input(&root_value, cx);
        window.click("nav-settings", cx);
        scroll_to(window, "settings-scroll", "settings-low", cx);
        window.click("settings-low", cx);
        window.press("ctrl-a", cx);
        window.input("15.000000001", cx);
        scroll_to(window, "settings-scroll", "settings-exclusions", cx);
        window.click("settings-exclusions", cx);
        window.press("ctrl-a", cx);
        window.input(&root_value, cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, cx| {
            assert_eq!(app.root.read(cx).value(), root_value);
            assert_eq!(app.low.read(cx).value(), "15.000000001");
            assert_eq!(app.runtime.view().settings.low_space_bytes, 15_000_000_001);
            assert_eq!(app.exclusions.read(cx).value(), root_value);
            assert_eq!(app.runtime.view().settings.map_depth, 3);
            assert!(app.runtime.view().settings.show_hidden);
            assert_eq!(app.runtime.view().map_color, 0);
            assert!(!app.runtime.view().map_isolate);
            assert!(!app.runtime.view().map_global);
        })
        .unwrap();
}

#[gpui_kit::test]
fn port_help_blocks_map_shortcuts_and_escape_closes_overlay(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
    idle(&mut runtime);
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-map", cx);
        window.click("map-viewport", cx);
        window.press("f1", cx);
        assert!(window.find("keys-dialog").visible());
        window.press("h", cx);
        window.press("a", cx);
        window.press("]", cx);
        window.press("escape", cx);
        assert!(window.try_find("keys-dialog").is_none());
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert!(!app.keys_open);
            assert!(app.runtime.view().settings.show_hidden);
            assert_eq!(app.runtime.view().map_color, 0);
            assert_eq!(app.runtime.view().settings.map_depth, 3);
        })
        .unwrap();
}

#[gpui_kit::test]
fn panel_upstream_mark_hidden_and_metric_keys_dispatch_from_canvas(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    let runtime = scanned_runtime(fixture.path(), "en", 100);
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-map", cx);
        window.click("map-viewport", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, cx| {
            let index = app
                .runtime
                .view()
                .map_objects
                .iter()
                .find(|r| r.path.ends_with("\\alpha"))
                .unwrap()
                .key
                .parse()
                .unwrap();
            app.focused_tile = Some(index);
            app.keyboard_target = true;
            app.dispatch(Command::MapFocus(index), cx);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("x", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(
                app.runtime.view().selected_count,
                1,
                "x marks the focused object"
            )
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("i", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert!(
                !app.runtime.view().settings.show_hidden,
                "i controls hidden projection"
            );
            assert!(!app.runtime.view().map_isolate, "i must not isolate");
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("t", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().map_metric, 2, "t cycles to Files")
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("d", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().map_metric, 2, "d keeps the Files metric");
            assert_eq!(
                app.runtime.view().map_size_metric,
                1,
                "d changes the retained byte basis"
            );
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("t", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().map_color, 1, "t cycles to Age")
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("t", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().map_metric, 1);
            assert_eq!(app.runtime.view().map_color, 0);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("d", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(
                app.runtime.view().map_metric,
                0,
                "d switches back to allocated size"
            )
        })
        .unwrap();
}

#[gpui_kit::test]
fn panel_review_shortcuts_keep_marks_and_require_separate_confirmation(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    let mut runtime = scanned_runtime(fixture.path(), "en", 100);
    let file = fixture.path().join("scan").join("alpha").join("first.bin");
    runtime.command(Command::Mark(file.to_string_lossy().into_owned()));
    assert_eq!(
        runtime.view().selected_count,
        1,
        "review fixture is marked before opening UI"
    );
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-map", cx);
        window.click("map-viewport", cx);
        window.press("c", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(
                app.runtime.view().selected_count,
                1,
                "c opens review without clearing marks"
            );
            assert!(app.runtime.view().busy || app.runtime.view().review.is_some());
        })
        .unwrap();
    handle
        .update(cx, |app, _, cx| {
            idle(&mut app.runtime);
            cx.notify();
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("review-modal").visible());
        window.press("s", cx);
        window.press("a", cx);
    })
    .unwrap();
    assert!(
        cx.did_prompt_for_new_path(),
        "review s opens the existing TXT picker"
    );
    assert!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .is_some_and(|text| text.contains("first.bin")),
        "review a copies the selected prompt"
    );
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("enter", cx);
        assert!(window.find("permanent-dialog").visible());
        window.press("enter", cx);
        assert!(
            window.find("permanent-dialog").visible(),
            "held/repeated Enter never commits deletion"
        );
        window.press("escape", cx);
        assert!(window.try_find("permanent-dialog").is_none());
        assert!(window.find("review-modal").visible());
        window.press("m", cx);
        assert!(window.find("review-trash-unsupported").visible());
        window.press("p", cx);
        window.press("!", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().selected_count, 0);
            assert!(app.runtime.view().review.is_none());
        })
        .unwrap();
    assert_eq!(std::fs::metadata(file).unwrap().len(), 16384);
}

#[gpui_kit::test]
fn panel_minimum_sidebar_marks_forecast_and_recommendations_remain_reachable(
    cx: &mut TestAppContext,
) {
    let fixture = owned_fixture();
    cx.update(gpui_omarchy::init);
    for language in ["ru", "en"] {
        for scale in [75, 100, 150] {
            let mut runtime = scanned_runtime(fixture.path(), language, scale);
            runtime.command(Command::Mark(
                fixture
                    .path()
                    .join("scan")
                    .join("alpha")
                    .to_string_lossy()
                    .into_owned(),
            ));
            assert_eq!(
                runtime.view().selected_count,
                1,
                "sidebar fixture is marked before opening UI"
            );
            let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
                App::new(runtime, window, cx)
            });
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click("nav-map", cx);
                inside_minimum_window(window, "map-viewport");
                assert!(window.find("map-viewport").bounds().size.height >= px(180.));
                for target in [
                    "map-marked-summary",
                    "map-marked-root-0",
                    "map-clear-marks",
                    "map-forecast-summary",
                    "map-recommendations-title",
                ] {
                    scroll_to(window, "map-sidebar", target, cx);
                    inside_minimum_window(window, target);
                }
                scroll_to(window, "map-sidebar", "map-marked-root-0", cx);
                window.click("map-marked-root-0", cx);
            })
            .unwrap();
            handle
                .update(cx, |app, _, _| {
                    assert_eq!(app.runtime.view().selected_count, 0)
                })
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn panel_launch_opens_map_initially_and_after_queued_root_poll(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    let mut runtime = scanned_runtime(fixture.path(), "en", 100);
    let scan = fixture.path().join("scan");
    runtime
        .apply_launch(crate::cli::LaunchOverrides {
            root: Some(scan.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .unwrap();
    idle(&mut runtime);
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        inside_minimum_window(window, "map-viewport");
    })
    .unwrap();
    let next = scan.join("beta").to_string_lossy().into_owned();
    handle
        .update(cx, |app, _, cx| {
            assert_eq!(app.page, Page::Map);
            app.page = Page::Overview;
            app.runtime
                .queue_launch(crate::cli::LaunchOverrides {
                    root: Some(next.clone()),
                    ..Default::default()
                })
                .unwrap();
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    handle
        .update(cx, |app, _, cx| {
            assert_eq!(
                app.page,
                Page::Map,
                "the owned UI poll opens queued explicit launches"
            );
            assert_eq!(app.runtime.view().root, next);
            idle(&mut app.runtime);
            cx.notify();
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        inside_minimum_window(window, "map-viewport");
    })
    .unwrap();
}

#[gpui_kit::test]
fn panel_escape_clears_query_then_focus_then_ascends_without_touching_marks(
    cx: &mut TestAppContext,
) {
    let fixture = owned_fixture();
    let runtime = scanned_runtime(fixture.path(), "en", 100);
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-map", cx);
    })
    .unwrap();
    let alpha = fixture
        .path()
        .join("scan")
        .join("alpha")
        .to_string_lossy()
        .into_owned();
    handle
        .update(cx, |app, _, cx| {
            let index = app
                .runtime
                .view()
                .map_objects
                .iter()
                .find(|row| row.path == alpha)
                .unwrap()
                .key
                .parse()
                .unwrap();
            app.dispatch(Command::MapNavigate(index), cx);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("map-viewport", cx);
        window.press("x", cx);
        window.press("s", cx);
        window.input("first", cx);
        window.press("escape", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert!(app.runtime.view().map_query.is_empty());
            assert_eq!(app.runtime.view().map_path, alpha);
            assert_eq!(app.runtime.view().selected_count, 1);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert!(app.active_tile().is_none());
            assert!(app.runtime.view().focused_path.is_empty());
            assert_eq!(app.runtime.view().map_path, alpha);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(
                app.runtime.view().map_path,
                fixture.path().join("scan").to_string_lossy()
            );
            assert_eq!(app.runtime.view().selected_count, 1);
        })
        .unwrap();
}

#[gpui_kit::test]
fn panel_covered_parent_unmark_action_and_hidden_marks_are_explicit(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    let alpha = fixture.path().join("scan").join("alpha");
    std::fs::create_dir_all(&alpha).unwrap();
    let path = crate::platform::wide(alpha.to_str().unwrap());
    let previous =
        unsafe { windows_sys::Win32::Storage::FileSystem::GetFileAttributesW(path.as_ptr()) };
    assert_ne!(
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                path.as_ptr(),
                previous | windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
            )
        },
        0
    );
    let mut runtime = scanned_runtime(fixture.path(), "en", 100);
    runtime.command(Command::Mark(alpha.to_string_lossy().into_owned()));
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-map", cx);
        window.click("map-viewport", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, cx| {
            let index = app
                .map_tiles
                .borrow()
                .iter()
                .find(|tile| {
                    tile.name == "first.bin"
                        && app
                            .runtime
                            .map_path(tile.index)
                            .is_some_and(|p| p.contains("\\alpha\\"))
                })
                .unwrap()
                .index;
            app.focused_tile = Some(index);
            app.keyboard_target = true;
            app.dispatch(Command::MapFocus(index), cx);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("space", cx);
        scroll_to(window, "map-sidebar", "map-unmark-parent", cx);
        inside_minimum_window(window, "map-unmark-parent");
        window.click("map-unmark-parent", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().selected_count, 0)
        })
        .unwrap();
    handle
        .update(cx, |app, _, cx| {
            app.dispatch(Command::Mark(alpha.to_string_lossy().into_owned()), cx);
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("map-viewport", cx);
        window.press("i", cx);
        scroll_to(window, "map-sidebar", "map-marked-root-0", cx);
        inside_minimum_window(window, "map-marked-root-0");
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().selected_count, 1);
            assert!(!app.runtime.view().settings.show_hidden);
            assert!(
                !app.map_tiles
                    .borrow()
                    .iter()
                    .any(|tile| tile.name == "alpha")
            );
            assert_eq!(
                app.runtime.view().map_marked[0].path,
                alpha.to_string_lossy()
            );
        })
        .unwrap();
    assert_ne!(
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(path.as_ptr(), previous)
        },
        0
    );
}

#[gpui_kit::test]
fn panel_sidebar_recommendation_click_navigates_without_marks_or_review(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    let cache = fixture.path().join("scan").join(".cache");
    std::fs::create_dir_all(&cache).unwrap();
    let file = cache.join("large-owned-fixture.bin");
    std::fs::File::create(&file)
        .unwrap()
        .set_len(72 * 1024 * 1024)
        .unwrap();
    let runtime = scanned_runtime(fixture.path(), "en", 100);
    assert_eq!(runtime.view().recommendations.len(), 1);
    cx.update(gpui_omarchy::init);
    let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
        App::new(runtime, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-map", cx);
        scroll_to(window, "map-sidebar", "map-recommendation-0", cx);
        window.click("map-recommendation-0", cx);
    })
    .unwrap();
    handle
        .update(cx, |app, _, _| {
            assert_eq!(app.runtime.view().map_path, cache.to_string_lossy());
            assert_eq!(app.runtime.view().selected_count, 0);
            assert!(app.runtime.view().review.is_none());
        })
        .unwrap();
    assert!(file.exists());
}

#[gpui_kit::test]
fn panel_modal_backdrops_block_canvas_mouse_marks_navigation_and_scroll(cx: &mut TestAppContext) {
    let fixture = owned_fixture();
    cx.update(gpui_omarchy::init);
    for permanent in [false, true] {
        let mut runtime = scanned_runtime(fixture.path(), "en", 100);
        let selected = fixture.path().join("scan").join("alpha").join("first.bin");
        runtime.command(Command::Mark(selected.to_string_lossy().into_owned()));
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            App::new(runtime, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-map", cx);
            window.click("map-viewport", cx);
            window.press("c", cx);
        })
        .unwrap();
        handle
            .update(cx, |app, _, cx| {
                idle(&mut app.runtime);
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            if permanent {
                window.press("enter", cx);
            }
            let viewport = window.find("map-viewport").bounds();
            let at = point(viewport.left() + px(4.), viewport.bottom() - px(4.));
            let footer = window
                .find(if permanent {
                    "permanent-cancel"
                } else {
                    "review-close"
                })
                .bounds();
            assert!(
                at.y > footer.bottom() + px(16.),
                "fixture must click below the popup footer and padding: viewport={viewport:?}, footer={footer:?}, at={at:?}"
            );
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
                    button: gpui_kit::MouseButton::Left,
                    position: at,
                    click_count: 1,
                    modifiers: gpui_kit::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
                    button: gpui_kit::MouseButton::Navigate(gpui_kit::NavigationDirection::Back),
                    position: at,
                    click_count: 1,
                    modifiers: Default::default(),
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                gpui_kit::PlatformInput::ScrollWheel(gpui_kit::ScrollWheelEvent {
                    position: at,
                    delta: gpui_kit::ScrollDelta::Lines(point(0., 2.)),
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        handle
            .update(cx, |app, _, _| {
                assert_eq!(
                    app.runtime.view().selected_count,
                    1,
                    "modal mouse input cannot change marks"
                );
                assert!(
                    app.runtime.view().review.is_some(),
                    "modal mouse input cannot dismiss its review"
                );
                assert_eq!(
                    app.runtime.view().map_path,
                    fixture.path().join("scan").to_string_lossy()
                );
                assert_eq!(
                    app.transform.scale, 1.,
                    "modal scroll cannot change map zoom"
                );
                assert_eq!(app.confirmation.is_open(), permanent);
            })
            .unwrap();
        assert_eq!(std::fs::metadata(selected).unwrap().len(), 16384);
    }
}
