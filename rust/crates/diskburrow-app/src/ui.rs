//! Six native pages. Every operation is dispatched to the UI-thread runtime.
use crate::{
    contract::{Command, MapTile, Page, Row, Setting, UiView},
    input::ConfirmationGate,
    map_view::{self, ViewTransform},
    runtime::Runtime,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::base::{
    Button, Checkbox, FocusTrapElement as _, Input as NativeInput, InputBase, Textarea,
    input::{InputEditorStyle, InputEvent, InputState, TextareaState},
};
use gpui_kit::{
    AppContext as _, Bounds, Context, Div, Entity, FocusHandle, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels, Render,
    Stateful, StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _,
    px, rgb, rgba, uniform_list,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Hsla,
    pub panel: Hsla,
    pub inset: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub danger: Hsla,
}
impl Palette {
    pub fn new(theme: &str) -> Self {
        if theme == "dark" {
            Self {
                background: rgb(0x111820).into(),
                panel: rgb(0x182330).into(),
                inset: rgb(0x0c141d).into(),
                text: rgb(0xe4edf4).into(),
                muted: rgb(0x9caebe).into(),
                border: rgb(0x304354).into(),
                accent: rgb(0x6cb8ff).into(),
                danger: rgb(0xfb9c85).into(),
            }
        } else {
            Self {
                background: rgb(0xf4f5f0).into(),
                panel: rgb(0xffffff).into(),
                inset: rgb(0xe9eee7).into(),
                text: rgb(0x202e31).into(),
                muted: rgb(0x63746e).into(),
                border: rgb(0xccd7d0).into(),
                accent: rgb(0x1769aa).into(),
                danger: rgb(0xa13923).into(),
            }
        }
    }
    pub fn editor(self) -> InputEditorStyle {
        InputEditorStyle {
            foreground: self.text,
            muted_foreground: self.muted,
            background: self.panel,
            border: self.border,
            selection: self.accent.opacity(0.25),
            caret: self.accent,
            ..Default::default()
        }
    }
}
#[derive(Clone, Copy)]
enum TableKind {
    Info,
    Mark,
    Cleanup,
    History,
    Map,
    Insight,
}

pub struct App {
    pub runtime: Runtime,
    pub page: Page,
    pub map_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub map_tiles: Rc<RefCell<Vec<MapTile>>>,
    pub transform: ViewTransform,
    pub map_focus: FocusHandle,
    pub map_drag: Option<(f32, f32)>,
    pub focused_tile: Option<usize>,
    pub pointer_tile: Option<usize>,
    pub keyboard_target: bool,
    pub sidebar_drag: bool,
    sidebar_drag_start: Option<(f32, f32)>,
    displayed_scan_id: Option<uuid::Uuid>,
    keys_open: bool,
    details_open: bool,
    trash_notice: bool,
    siblings_open: Option<usize>,
    base_rem: f32,
    crumbs_scroll: gpui_kit::ScrollHandle,
    crumbs_path: String,
    system_dark: Option<bool>,
    appearance_polled: std::time::Instant,
    root: Entity<InputState>,
    search: Entity<InputState>,
    filter: Entity<InputState>,
    low: Entity<InputState>,
    growth: Entity<InputState>,
    custom: Entity<InputState>,
    exclusions: Entity<TextareaState>,
    dialog_focus: FocusHandle,
    confirmation: ConfirmationGate,
    export_disclosure: bool,
    volumes_open: bool,
    largest_files: bool,
    history_changes: bool,
    cleanup_tab: u8,
    last_root: String,
}
impl App {
    pub fn new(mut runtime: Runtime, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = if runtime.take_map_open() {
            Page::Map
        } else {
            Page::Overview
        };
        let view = runtime.view();
        let settings = &view.settings;
        let root = cx.new(|cx| InputState::new(window, cx).default_value(view.root.clone()));
        let search = cx.new(|cx| InputState::new(window, cx));
        let filter = cx.new(|cx| InputState::new(window, cx));
        let low = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(crate::locale::threshold(settings.low_space_bytes))
        });
        let growth = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(crate::locale::threshold(settings.growth_bytes))
        });
        let custom = cx.new(|cx| {
            InputState::new(window, cx).default_value(
                settings
                    .approved_custom_temp_path
                    .clone()
                    .unwrap_or_default(),
            )
        });
        let exclusions = cx.new(|cx| {
            TextareaState::new(window, cx).default_value(settings.excluded_paths.join("\n"))
        });
        for (field, setting) in [
            (&low, Setting::LowSpaceGb),
            (&growth, Setting::GrowthGb),
            (&custom, Setting::CustomTemp),
        ] {
            cx.subscribe(field, move |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.dispatch(
                        Command::Setting(setting, state.read(cx).value().to_string()),
                        cx,
                    );
                }
            })
            .detach();
        }
        cx.subscribe(&exclusions, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dispatch(
                    Command::Setting(Setting::Exclusions, state.read(cx).value().to_string()),
                    cx,
                );
            }
        })
        .detach();
        cx.subscribe(&root, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = state.read(cx).value().to_string();
                this.last_root = value.clone();
                this.dispatch(Command::SetRoot(value), cx);
            }
        })
        .detach();
        cx.subscribe(&search, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dispatch(Command::MapSearch(state.read(cx).value().to_string()), cx);
            }
        })
        .detach();
        cx.subscribe(&filter, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dispatch(
                    Command::CleanupFilter(state.read(cx).value().to_string()),
                    cx,
                );
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let mut changed = this.runtime.poll();
                        if this.runtime.take_map_open() {
                            this.page = Page::Map;
                            changed = true;
                        }
                        if this.displayed_scan_id != this.runtime.scan_id() {
                            this.reset_map_targets();
                            changed = true;
                        }
                        if this.appearance_polled.elapsed() >= Duration::from_secs(2) {
                            let dark = crate::appearance::system_dark();
                            changed |= dark != this.system_dark;
                            this.system_dark = dark;
                            this.appearance_polled = std::time::Instant::now();
                        }
                        if this.runtime.take_show() {
                            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                            if let Ok(handle) = window.window_handle()
                                && let RawWindowHandle::Win32(handle) = handle.as_raw()
                            {
                                crate::platform::restore_own_window(handle.hwnd.get());
                            }
                            window.activate_window();
                        }
                        if this.runtime.should_exit() {
                            cx.quit();
                        }
                        if this.runtime.view().review.is_none() {
                            this.confirmation.dismiss();
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            last_root: view.root.clone(),
            displayed_scan_id: runtime.scan_id(),
            runtime,
            page,
            root,
            search,
            filter,
            low,
            growth,
            custom,
            exclusions,
            map_bounds: Rc::new(Cell::new(Bounds::default())),
            map_tiles: Rc::new(RefCell::new(vec![])),
            transform: ViewTransform::default(),
            map_focus: cx.focus_handle(),
            map_drag: None,
            focused_tile: None,
            pointer_tile: None,
            keyboard_target: true,
            sidebar_drag: false,
            sidebar_drag_start: None,
            keys_open: false,
            details_open: true,
            trash_notice: false,
            siblings_open: None,
            base_rem: window.rem_size().as_f32(),
            crumbs_scroll: gpui_kit::ScrollHandle::new(),
            crumbs_path: String::new(),
            system_dark: crate::appearance::system_dark(),
            appearance_polled: std::time::Instant::now(),
            dialog_focus: cx.focus_handle(),
            confirmation: ConfirmationGate::default(),
            export_disclosure: false,
            volumes_open: false,
            largest_files: false,
            history_changes: false,
            cleanup_tab: 0,
        }
    }
    fn reset_map_targets(&mut self) {
        self.transform = ViewTransform::default();
        self.runtime.command(Command::MapZoom(1.));
        self.focused_tile = None;
        self.pointer_tile = None;
        self.keyboard_target = true;
        self.map_drag = None;
        self.sidebar_drag = false;
        self.sidebar_drag_start = None;
        self.siblings_open = None;
        self.map_tiles.borrow_mut().clear();
    }
    pub fn dispatch(&mut self, command: Command, cx: &mut Context<Self>) {
        // Old-frame closures may run after publication but before the new frame.
        // Keep their generation stale until render rebuilds the view and hit rectangles.
        if self.displayed_scan_id != self.runtime.scan_id()
            && matches!(
                command,
                Command::MapMark(_) | Command::MapNavigate(_) | Command::MapFocus(_)
            )
        {
            self.reset_map_targets();
            cx.notify();
            return;
        }
        self.confirmation.dismiss();
        self.trash_notice = false;
        if matches!(
            command,
            Command::SetRoot(_)
                | Command::Scan(_)
                | Command::MapNavigate(_)
                | Command::MapBack
                | Command::MapForward
                | Command::MapUp
                | Command::MapRoot
                | Command::MapMetric(_)
                | Command::MapGlobal(_)
                | Command::MapIsolate(_)
                | Command::MapWiden(_)
                | Command::Setting(Setting::MapDepth | Setting::ShowHidden, _)
        ) {
            self.transform = ViewTransform::default();
            self.focused_tile = None;
            self.pointer_tile = None;
            self.siblings_open = None;
        }
        self.runtime.command(command);
        cx.notify();
    }
    fn text(&self, key: &str) -> String {
        crate::locale::text(&self.runtime.view().settings.language, key)
    }
    fn palette(&self) -> Palette {
        Palette::new(crate::appearance::resolve(
            &self.runtime.view().settings.theme,
            self.system_dark,
        ))
    }
    fn scaled(&self, value: f32) -> Pixels {
        px(value * self.runtime.view().settings.ui_scale_percent as f32 / 100.)
    }
    fn interface_zoom(&mut self, direction: i8, cx: &mut Context<Self>) {
        let steps = [75, 90, 100, 110, 125, 150];
        let current = steps
            .iter()
            .position(|s| *s == self.runtime.view().settings.ui_scale_percent)
            .unwrap_or(2);
        let next = if direction == 0 {
            2
        } else {
            (current as i8 + direction).clamp(0, 5) as usize
        };
        self.dispatch(
            Command::Setting(Setting::UiScale, steps[next].to_string()),
            cx,
        );
    }
    fn editor_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        [
            &self.root,
            &self.search,
            &self.filter,
            &self.low,
            &self.growth,
            &self.custom,
        ]
        .iter()
        .any(|state| state.focus_handle(cx).is_focused(window))
            || self.exclusions.focus_handle(cx).is_focused(window)
    }
    pub(crate) fn modal_open(&self) -> bool {
        self.confirmation.is_open()
            || self.runtime.view().review.is_some()
            || self.keys_open
            || self.siblings_open.is_some()
            || self.export_disclosure
            || self.volumes_open
    }
    fn request_permanent_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.runtime.view().busy
            && let Some(review) = self.runtime.view().review.clone()
        {
            self.confirmation
                .request(&review.id, review.cleanup, review.can_confirm);
            window.focus(&self.dialog_focus, cx);
            cx.notify();
        }
    }
    fn map_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.runtime.view().map_query.is_empty() || !self.search.read(cx).value().is_empty() {
            self.search
                .update(cx, |state, cx| state.set_value("", window, cx));
            self.dispatch(Command::MapSearch(String::new()), cx);
        } else if self.runtime.view().busy {
            self.dispatch(Command::Cancel, cx);
        } else if self.active_tile().is_some() || !self.runtime.view().focused_path.is_empty() {
            self.focused_tile = None;
            self.pointer_tile = None;
            self.keyboard_target = true;
            self.dispatch(Command::MapDismissFocus, cx);
        } else {
            self.dispatch(Command::MapUp, cx);
        }
        window.focus(&self.map_focus, cx);
    }
    fn global_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            if self.confirmation.is_open() {
                self.confirmation.dismiss();
            } else if self.keys_open {
                self.keys_open = false;
            } else if self.siblings_open.is_some() {
                self.siblings_open = None;
            } else if self.export_disclosure {
                self.export_disclosure = false;
            } else if self.volumes_open {
                self.volumes_open = false;
            } else if self.runtime.view().review.is_some() {
                self.dispatch(Command::DismissReview, cx);
            } else if self.page == Page::Map {
                self.map_escape(window, cx);
            } else {
                return;
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.confirmation.is_open()
            || self.keys_open
            || self.siblings_open.is_some()
            || self.export_disclosure
            || self.volumes_open
        {
            return;
        }
        if self.runtime.view().review.is_some() {
            if self.editor_focused(window, cx)
                || event.keystroke.modifiers.control
                || event.keystroke.modifiers.alt
            {
                return;
            }
            match key {
                "enter" => self.request_permanent_confirmation(window, cx),
                "s" if self.runtime.view().selected_count > 0 && !self.runtime.view().busy => {
                    self.export_marked(cx)
                }
                "a" if self.runtime.view().selected_count > 0 => self.copy_marked(cx),
                "!" => self.dispatch(Command::ClearMarks, cx),
                "p" => self.trash_notice = false,
                "m" => self.trash_notice = true,
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if event.keystroke.modifiers.control {
            match key {
                "+" | "=" => self.interface_zoom(1, cx),
                "-" => self.interface_zoom(-1, cx),
                "0" => self.interface_zoom(0, cx),
                "o" => self.choose_folder(cx),
                _ => return,
            }
            cx.stop_propagation();
        } else if self.page == Page::Map && !self.editor_focused(window, cx) {
            self.map_key(event, window, cx);
        } else if key == "f5" && !self.runtime.view().busy {
            self.dispatch(Command::Scan(false), cx);
            cx.stop_propagation();
        }
    }
    pub(crate) fn active_tile(&self) -> Option<usize> {
        if self.keyboard_target {
            self.focused_tile
        } else {
            self.pointer_tile.or(self.focused_tile)
        }
    }
    fn export_marked(&mut self, cx: &mut Context<Self>) {
        let directory = PathBuf::from(&self.runtime.view().root);
        let selected = cx.prompt_for_new_path(&directory, Some("DiskBurrow-selected.txt"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = selected.await {
                let _ = this.update(cx, |this, cx| {
                    this.dispatch(
                        Command::ExportSelection(path.to_string_lossy().into_owned()),
                        cx,
                    )
                });
            }
        })
        .detach();
    }
    fn copy_marked(&mut self, cx: &mut Context<Self>) {
        if let Some(export) = self
            .runtime
            .selected_export()
            .filter(|export| export.count > 0)
        {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(export.prompt));
        }
    }
    fn button(
        &self,
        id: &str,
        label: String,
        command: Command,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        let p = self.palette();
        Button::new(id.to_owned())
            .accessibility_label(label.clone())
            .disabled(!enabled)
            .px(px(8.))
            .h(self.scaled(32.))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .text_color(if enabled { p.text } else { p.muted })
            .text_size(self.scaled(12.))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.dispatch(command.clone(), cx)))
    }
    fn local_button(
        &self,
        id: &str,
        label: String,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> Button {
        let p = self.palette();
        Button::new(id.to_owned())
            .accessibility_label(label.clone())
            .px(px(8.))
            .h(self.scaled(32.))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .text_color(p.text)
            .text_size(self.scaled(12.))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| action(this, window, cx)))
    }
    fn checkbox(
        &self,
        id: &str,
        label: String,
        checked: bool,
        command: Command,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Checkbox {
        let p = self.palette();
        let app = cx.entity().downgrade();
        Checkbox::new(id.to_owned())
            .accessibility_label(label.clone())
            .checked(checked)
            .disabled(!enabled)
            .flex()
            .items_center()
            .gap_2()
            .text_size(self.scaled(12.))
            .text_color(p.text)
            .child(
                div()
                    .w(px(18.))
                    .h(px(18.))
                    .border_1()
                    .border_color(p.border)
                    .bg(if checked { p.accent } else { p.panel })
                    .text_color(p.background)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if checked { "×" } else { "" }),
            )
            .child(label)
            .on_change(move |_, _, _, cx| {
                let _ = app.update(cx, |this, cx| this.dispatch(command.clone(), cx));
            })
    }
    fn edit(
        &self,
        id: &str,
        state: &Entity<InputState>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = self.palette();
        state.update(cx, |state, _| state.set_editor_style(p.editor()));
        let focus = state.read(cx).focus_handle(cx).is_focused(window);
        let field = state.clone();
        InputBase::new(id.to_owned())
            .focused(focus)
            .h(self.scaled(33.))
            .min_w_0()
            .w_full()
            .px_2()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .text_color(p.text)
            .text_size(self.scaled(13.))
            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                field.update(cx, |state, cx| state.focus(window, cx))
            })
            .child(NativeInput::new(state))
    }
    fn panel(&self, id: &str) -> Stateful<Div> {
        let p = self.palette();
        div()
            .id(id.to_owned())
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .min_w_0()
            .min_h_0()
    }
    fn note(&self, text: String) -> Div {
        div()
            .text_size(self.scaled(12.))
            .text_color(self.palette().muted)
            .child(text)
    }
    fn title(&self, key: &str) -> Div {
        div()
            .text_size(self.scaled(20.))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .child(self.text(key))
    }
    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.text("Action.Choose").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().into_owned();
                let _ = this.update(cx, |this, cx| this.dispatch(Command::SetRoot(path), cx));
            }
        })
        .detach();
    }
    fn export(&mut self, cx: &mut Context<Self>) {
        self.export_disclosure = false;
        let directory = PathBuf::from(&self.runtime.view().root);
        let selected = cx.prompt_for_new_path(&directory, Some("DiskBurrow-report.json"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = selected.await {
                let _ = this.update(cx, |this, cx| {
                    this.dispatch(Command::Export(path.to_string_lossy().into_owned()), cx)
                });
            }
        })
        .detach();
        cx.notify();
    }
    fn top(
        &self,
        view: &UiView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = self.palette();
        let root = self.edit("scan-root", &self.root, window, cx);
        div()
            .id("app-top")
            .test_support()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(6.))
            .p(px(8.))
            .bg(p.panel)
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(145.))
                            .flex_shrink_0()
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_size(self.scaled(19.))
                            .child("DiskBurrow"),
                    )
                    .child(div().flex_1().min_w_0().child(root))
                    .child(self.local_button(
                        "choose-folder",
                        self.text("Action.Choose"),
                        |this, _, cx| this.choose_folder(cx),
                        cx,
                    ))
                    .child(self.local_button(
                        "choose-drive",
                        self.text("Map.Root"),
                        |this, window, cx| {
                            this.volumes_open = true;
                            window.focus(&this.dialog_focus, cx);
                            cx.notify();
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(self.button(
                        "scan-normal",
                        self.text("Action.Scan"),
                        Command::Scan(false),
                        !view.busy,
                        cx,
                    ))
                    .child(gpui_omarchy::with_tooltip(
                        self.button(
                            "scan-fast",
                            self.text("Action.ScanFast"),
                            Command::Scan(true),
                            !view.busy,
                            cx,
                        ),
                        self.text("FastScan.Hint"),
                    ))
                    .child(self.button(
                        "cancel-operation",
                        self.text("Action.Cancel"),
                        Command::Cancel,
                        view.busy,
                        cx,
                    ))
                    .child(self.checkbox(
                        "pause-background",
                        self.text("Action.Pause"),
                        view.paused,
                        Command::Pause,
                        true,
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(self.local_button(
                        "export-report",
                        self.text("Action.Export"),
                        |this, window, cx| {
                            this.export_disclosure = true;
                            window.focus(&this.dialog_focus, cx);
                            cx.notify();
                        },
                        cx,
                    )),
            )
    }
    fn navigation(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette();
        let mut nav = div()
            .w(px(154.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(p.panel)
            .border_r_1()
            .border_color(p.border);
        for (page, id, key) in [
            (Page::Overview, "nav-overview", "Nav.Overview"),
            (Page::Largest, "nav-largest", "Nav.Largest"),
            (Page::Map, "nav-map", "Nav.Map"),
            (Page::History, "nav-history", "Nav.History"),
            (Page::Cleanup, "nav-cleanup", "Nav.Cleanup"),
            (Page::Settings, "nav-settings", "Nav.Settings"),
        ] {
            let label = self.text(key);
            nav = nav.child(
                Button::new(id)
                    .accessibility_label(label.clone())
                    .w_full()
                    .min_h(px(44.))
                    .px_3()
                    .py_2()
                    .text_size(self.scaled(12.))
                    .bg(if self.page == page { p.inset } else { p.panel })
                    .text_color(if self.page == page { p.accent } else { p.text })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = page;
                        this.confirmation.dismiss();
                        cx.notify();
                    })),
            );
        }
        nav.child(div().flex_1())
            .child(self.note(format!("{} · Rust / GPUI", env!("CARGO_PKG_VERSION"))))
    }
    fn table(
        &self,
        id: &str,
        rows: Vec<Row>,
        headers: &[&str],
        kind: TableKind,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = self.palette();
        let widths: Vec<f32> = headers
            .iter()
            .map(|key| match *key {
                "Modified" | "Scan.Time" => 185.,
                "Reason" => 160.,
                "Category" => 125.,
                "Outcome" | "Kind" => 125.,
                "Logical" | "Allocated" => 100.,
                _ => 86.,
            })
            .collect();
        let is_select = matches!(kind, TableKind::Mark | TableKind::Cleanup);
        let mut header = div()
            .id(format!("{id}-header"))
            .test_support()
            .flex()
            .gap_2()
            .items_center()
            .h(px(30.))
            .flex_shrink_0()
            .px_2()
            .bg(p.inset)
            .text_size(self.scaled(11.))
            .text_color(p.muted);
        if is_select {
            header = header.child(div().w(px(56.)).flex_shrink_0().child(self.text("Select")));
        }
        header = header.child(div().flex_1().min_w_0().child(self.text(
            if id == "overview-issues" {
                "Reason"
            } else {
                "Path"
            },
        )));
        if matches!(kind, TableKind::Cleanup) {
            header = header.child(div().w(px(20.)).flex_shrink_0());
        }
        for (column, key) in headers.iter().enumerate() {
            header = header.child(
                div()
                    .w(px(widths[column]))
                    .flex_shrink_0()
                    .text_ellipsis()
                    .child(self.text(key)),
            );
        }
        let row_id = id.to_owned();
        let count = rows.len();
        let rows = Rc::new(rows);
        let list = uniform_list(
            id.to_owned(),
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|i| {
                        let row = &rows[i];
                        let mut item = div()
                            .id(format!("{}-{}", row_id, i))
                            .role(gpui_kit::Role::Row)
                            .test_support()
                            .aria_label(row.path.clone())
                            .h(this.scaled(34.))
                            .px_2()
                            .flex()
                            .gap_2()
                            .items_center()
                            .border_b_1()
                            .border_color(p.border.opacity(0.5))
                            .bg(if row.selected {
                                p.accent.opacity(0.09)
                            } else {
                                p.panel
                            })
                            .text_size(this.scaled(12.))
                            .overflow_hidden();
                        if is_select {
                            let command = if matches!(kind, TableKind::Cleanup) {
                                Command::ToggleCandidate(row.key.clone())
                            } else {
                                Command::Mark(row.path.clone())
                            };
                            item = item.child(
                                this.checkbox(
                                    &format!("{}-check-{}", row_id, i),
                                    String::new(),
                                    row.selected,
                                    command,
                                    row.selectable,
                                    cx,
                                )
                                .w(px(56.))
                                .flex_shrink_0(),
                            );
                        }
                        let mut path = gpui_omarchy::with_tooltip(
                            div()
                                .id(format!("{}-path-{}", row_id, i))
                                .role(gpui_kit::Role::Link)
                                .test_support()
                                .aria_label(row.path.clone())
                                .when(matches!(kind, TableKind::Insight), |path| {
                                    path.cursor_pointer().text_color(p.accent)
                                })
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis_middle()
                                .child(row.path.clone()),
                            row.path.clone(),
                        );
                        match kind {
                            TableKind::Insight => {
                                if let Ok(index) = row.key.parse::<usize>() {
                                    path =
                                        path.on_click(cx.listener(move |this, _, window, cx| {
                                            this.page = Page::Map;
                                            this.focused_tile = Some(index);
                                            this.dispatch(Command::MapNavigate(index), cx);
                                            this.dispatch(Command::MapFocus(index), cx);
                                            window.focus(&this.map_focus, cx);
                                        }));
                                }
                            }
                            TableKind::History => {
                                let key = row.key.clone();
                                path = path.on_click(cx.listener(move |this, _, _, cx| {
                                    this.dispatch(Command::SelectHistory(key.clone()), cx)
                                }));
                            }
                            TableKind::Map => {
                                if let Ok(index) = row.key.parse::<usize>() {
                                    path = path.on_click(cx.listener(
                                        move |this, event: &gpui_kit::ClickEvent, window, cx| {
                                            this.focused_tile = Some(index);
                                            this.dispatch(
                                                if event.click_count() == 2 {
                                                    Command::MapNavigate(index)
                                                } else {
                                                    Command::MapFocus(index)
                                                },
                                                cx,
                                            );
                                            window.focus(&this.map_focus, cx);
                                        },
                                    ));
                                }
                            }
                            _ => {
                                let target = row.path.clone();
                                path = path.on_click(cx.listener(
                                    move |this, event: &gpui_kit::ClickEvent, _, cx| {
                                        if event.click_count() == 2 {
                                            this.dispatch(Command::Open(target.clone()), cx);
                                        }
                                    },
                                ));
                            }
                        }
                        item = item.child(path);
                        if matches!(kind, TableKind::Cleanup) {
                            let key = row.key.clone();
                            item = item.child(
                                Button::new(format!("{}-exclude-{}", row_id, i))
                                    .accessibility_label(this.text("Action.ExcludeFile"))
                                    .disabled(!row.selectable)
                                    .w(px(20.))
                                    .h(px(24.))
                                    .flex_shrink_0()
                                    .text_color(p.danger)
                                    .child("−")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch(Command::ExcludeCandidate(key.clone()), cx)
                                    })),
                            );
                        }
                        for (column, _width) in widths.iter().enumerate() {
                            item = item.child(gpui_omarchy::with_tooltip(
                                div()
                                    .id(format!("{}-cell-{}-{}", row_id, i, column))
                                    .w(px(widths[column]))
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(row.cells.get(column).cloned().unwrap_or_default()),
                                row.cells.get(column).cloned().unwrap_or_default(),
                            ));
                        }
                        item
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .min_h_0()
        .w_full();
        div()
            .id(format!("{id}-container"))
            .test_support()
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .flex_1()
            .border_1()
            .border_color(p.border)
            .child(header)
            .child(if count == 0 {
                div()
                    .p_3()
                    .text_color(p.muted)
                    .child(self.text("Empty"))
                    .into_any_element()
            } else {
                list.into_any_element()
            })
    }
    fn overview(&self, view: &UiView, cx: &Context<Self>) -> impl IntoElement + use<> {
        let mut cards = div()
            .id("overview-cards")
            .test_support()
            .flex()
            .flex_wrap()
            .gap_3()
            .max_h(px(160.))
            .min_h_0()
            .flex_shrink_0()
            .overflow_y_scroll();
        for (index, (key, value)) in view.overview.iter().enumerate() {
            cards = cards.child(
                self.panel(&format!("overview-{index}"))
                    .min_w(px(165.))
                    .flex_1()
                    .child(self.note(crate::locale::text(&view.settings.language, key)))
                    .child(gpui_omarchy::with_tooltip(
                        div()
                            .id(format!("overview-value-{index}"))
                            .text_size(self.scaled(22.))
                            .text_ellipsis()
                            .child(value.clone()),
                        value.clone(),
                    )),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .size_full()
            .min_h_0()
            .child(self.title("Nav.Overview"))
            .child(cards)
            .child(gpui_omarchy::with_tooltip(
                div()
                    .id("volume-stamp")
                    .h(px(18.))
                    .text_size(self.scaled(11.))
                    .text_ellipsis()
                    .child(view.volume_stamp.clone()),
                view.volume_stamp.clone(),
            ))
            .child(self.note(self.text("Overview.Explanation")))
            .child(self.table(
                "overview-issues",
                view.issue_rows.clone(),
                &["Issue.Count"],
                TableKind::Info,
                cx,
            ))
    }
    fn largest(&self, view: &UiView, cx: &Context<Self>) -> impl IntoElement + use<> {
        let rows = if self.largest_files {
            view.files.clone()
        } else {
            view.folders.clone()
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .min_h_0()
            .child(self.title("Nav.Largest"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.local_button(
                        "largest-folders",
                        self.text("Manual.Folders"),
                        |this, _, cx| {
                            this.largest_files = false;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.local_button(
                        "largest-files",
                        self.text("Manual.Files"),
                        |this, _, cx| {
                            this.largest_files = true;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(self.button(
                        "manual-review",
                        self.text("Manual.Analyze"),
                        Command::PreviewManual,
                        view.can_manual && !view.busy,
                        cx,
                    )),
            )
            .child(self.note(self.text("Manual.SelectionHelp")))
            .child(self.table(
                "largest-table",
                rows,
                &[
                    "Logical",
                    "Allocated",
                    if self.largest_files {
                        "Modified"
                    } else {
                        "Coverage"
                    },
                ],
                TableKind::Mark,
                cx,
            ))
            .child(self.note(format!(
                "{}: {} · {}",
                self.text("Manual.Selected"),
                view.selected_count,
                self.text("Lists.Bound")
            )))
    }
    fn history(&self, view: &UiView, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .min_h_0()
            .child(self.title("Nav.History"))
            .child(self.note(self.text("History.Sparse")))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.local_button(
                        "history-scans",
                        self.text("Overview.Checked"),
                        |this, _, cx| {
                            this.history_changes = false;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.local_button(
                        "history-changes",
                        self.text("Delta"),
                        |this, _, cx| {
                            this.history_changes = true;
                            cx.notify();
                        },
                        cx,
                    )),
            )
            .child(self.note(view.history_heading.clone()))
            .child(if self.history_changes {
                self.table(
                    "history-changes-table",
                    view.changes.clone(),
                    &["Delta", "Kind", "Comparable"],
                    TableKind::Info,
                    cx,
                )
                .into_any_element()
            } else {
                self.table(
                    "history-scans-table",
                    view.history.clone(),
                    &["Scan.Time", "Logical", "Coverage"],
                    TableKind::History,
                    cx,
                )
                .into_any_element()
            })
    }
    fn map_legend(&self) -> Div {
        let mut legend = div()
            .id("map-category-legend")
            .flex()
            .flex_wrap()
            .gap_1()
            .text_size(self.scaled(10.));
        if self.runtime.view().map_color == 1 {
            for (key, color) in [
                ("Age.Unknown", 0x697784),
                ("Age.7", 0x397c70),
                ("Age.30", 0x487caf),
                ("Age.180", 0x8a8545),
                ("Age.365", 0xac733e),
                ("Age.Older", 0x8f4b62),
            ] {
                legend = legend.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(div().w(px(7.)).h(px(7.)).bg(rgb(color)))
                        .child(self.text(key)),
                );
            }
        } else {
            for category in crate::recommendations::legend() {
                legend = legend.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .w(px(7.))
                                .h(px(7.))
                                .bg(map_view::category_color(category)),
                        )
                        .child(crate::locale::text(
                            &self.runtime.view().settings.language,
                            crate::recommendations::category_key(category),
                        )),
                );
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(legend)
            .when(self.runtime.view().map_color == 1, |element| {
                element.child(self.note(self.text("Age.Note")))
            })
            .child(gpui_omarchy::with_tooltip(
                div()
                    .id("map-hatch-note")
                    .text_size(self.scaled(10.))
                    .text_ellipsis()
                    .child(self.text("Reclaim.Hatch")),
                self.text("Reclaim.Hatch"),
            ))
    }
    fn cleanup(
        &self,
        view: &UiView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let filter = self.edit("cleanup-filter", &self.filter, window, cx);
        let mut categories = div().flex().flex_wrap().gap(px(4.));
        for (index, category) in view.cleanup_categories.iter().enumerate() {
            let label = if category.is_empty() {
                self.text("All")
            } else {
                crate::locale::text(&view.settings.language, category)
            };
            categories = categories.child(self.button(
                &format!("cleanup-category-{index}"),
                label,
                Command::CleanupCategory(category.clone()),
                !view.busy,
                cx,
            ));
        }
        let body = match self.cleanup_tab {
            4 => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .id("insights-heading")
                        .role(gpui_kit::Role::Heading)
                        .test_support()
                        .aria_label(self.text("Insights.Title"))
                        .child(self.text("Insights.Title")),
                )
                .child(gpui_omarchy::with_tooltip(
                    div()
                        .id("insights-note")
                        .text_size(self.scaled(11.))
                        .text_ellipsis()
                        .child(self.text("Insights.Note")),
                    self.text("Insights.Note"),
                ))
                .child(self.table(
                    "insights-list",
                    view.recommendations.clone(),
                    &["Category", "Reason", "Allocated"],
                    TableKind::Insight,
                    cx,
                ))
                .into_any_element(),
            3 => {
                let mut buttons = div().flex().flex_wrap().gap(px(4.));
                for (index, label) in [
                    self.text("Action.Storage"),
                    self.text("Recommend.Chrome"),
                    self.text("Recommend.Edge"),
                    "NuGet".into(),
                    "pip".into(),
                ]
                .into_iter()
                .enumerate()
                {
                    buttons = buttons.child(self.button(
                        &format!("maintenance-{index}"),
                        label,
                        Command::Maintenance(index),
                        true,
                        cx,
                    ));
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .flex_1()
                    .min_h_0()
                    .child(self.note(self.text("Recommend.Text")))
                    .child(buttons)
                    .child(gpui_omarchy::with_tooltip(
                        div()
                            .id("observed-cache-limit")
                            .h(px(18.))
                            .text_size(self.scaled(11.))
                            .text_ellipsis()
                            .child(self.text("Recommend.Limit")),
                        self.text("Recommend.Limit"),
                    ))
                    .child(self.table(
                        "observed-caches",
                        view.observed_caches.clone(),
                        &["Category", "Logical"],
                        TableKind::Info,
                        cx,
                    ))
                    .into_any_element()
            }
            1 => self
                .table(
                    "cleanup-warnings",
                    view.cleanup_warnings.clone(),
                    &["Category", "Reason"],
                    TableKind::Info,
                    cx,
                )
                .into_any_element(),
            2 => self
                .table(
                    "cleanup-results",
                    view.cleanup_results.clone(),
                    &["Outcome", "Reason"],
                    TableKind::Info,
                    cx,
                )
                .into_any_element(),
            _ => self
                .table(
                    "cleanup-candidates",
                    view.cleanup.clone(),
                    &["Logical", "Category", "Reason"],
                    TableKind::Cleanup,
                    cx,
                )
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .size_full()
            .min_h_0()
            .child(self.title("Nav.Cleanup"))
            .child(gpui_omarchy::with_tooltip(
                div()
                    .id("cleanup-scope")
                    .h(px(18.))
                    .text_size(self.scaled(11.))
                    .text_ellipsis()
                    .child(self.text("Cleanup.Scope")),
                self.text("Cleanup.Scope"),
            ))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(self.button(
                        "analyze-cleanup",
                        self.text("Action.Analyze"),
                        Command::AnalyzeCleanup,
                        !view.busy,
                        cx,
                    ))
                    .child(
                        self.local_button(
                            "review-cleanup",
                            self.text("Manual.Review"),
                            |this, window, cx| {
                                this.cleanup_tab = 0;
                                window.focus(&this.dialog_focus, cx);
                                this.dispatch(Command::PreviewCleanup, cx);
                            },
                            cx,
                        )
                        .disabled(!view.can_cleanup || view.busy),
                    )
                    .child(self.button(
                        "exclude-cleanup-category",
                        self.text("Action.ExcludeCategory"),
                        Command::ExcludeCategory,
                        !view.busy,
                        cx,
                    )),
            )
            .child(categories)
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .items_center()
                    .child(div().w(px(100.)).child(self.text("Filter")))
                    .child(div().flex_1().child(filter)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(self.local_button(
                        "cleanup-tab-candidates",
                        self.text("Cleanup.Count"),
                        |this, _, cx| {
                            this.cleanup_tab = 0;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.local_button(
                        "cleanup-tab-warnings",
                        self.text("Cleanup.Warnings"),
                        |this, _, cx| {
                            this.cleanup_tab = 1;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.local_button(
                        "cleanup-tab-results",
                        self.text("Outcome"),
                        |this, _, cx| {
                            this.cleanup_tab = 2;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.local_button(
                        "cleanup-tab-recommendations",
                        self.text("Insights.Title"),
                        |this, _, cx| {
                            this.cleanup_tab = 4;
                            cx.notify();
                        },
                        cx,
                    )),
            )
            .when(matches!(self.cleanup_tab, 3 | 4), |body| {
                body.child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(self.local_button(
                            "insights-worth-look",
                            self.text("Insights.Title"),
                            |this, _, cx| {
                                this.cleanup_tab = 4;
                                cx.notify();
                            },
                            cx,
                        ))
                        .child(self.local_button(
                            "insights-observed",
                            self.text("Insights.Observed"),
                            |this, _, cx| {
                                this.cleanup_tab = 3;
                                cx.notify();
                            },
                            cx,
                        )),
                )
            })
            .child(self.note(view.cleanup_summary.clone()))
            .child(body)
            .child(gpui_omarchy::with_tooltip(
                div()
                    .id("cleanup-bound")
                    .h(px(18.))
                    .text_size(self.scaled(11.))
                    .text_ellipsis()
                    .child(self.text("Cleanup.Bound")),
                self.text("Cleanup.Bound"),
            ))
    }
    fn settings(
        &self,
        view: &UiView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = self.palette();
        let mut interval = div().flex().gap_2();
        for hours in [1, 6, 12, 24] {
            interval = interval.child(self.button(
                &format!("interval-{hours}"),
                hours.to_string(),
                Command::Setting(Setting::Interval, hours.to_string()),
                true,
                cx,
            ));
        }
        let low = self.edit("settings-low", &self.low, window, cx);
        let growth = self.edit("settings-growth", &self.growth, window, cx);
        let custom = self.edit("settings-custom-temp", &self.custom, window, cx);
        let field = |id: &str, label: String, control: gpui_kit::AnyElement| {
            div()
                .id(id.to_owned())
                .flex()
                .items_start()
                .gap_3()
                .min_w_0()
                .child(
                    div()
                        .w(px(235.))
                        .flex_shrink_0()
                        .text_size(self.scaled(12.))
                        .child(label),
                )
                .child(div().flex_1().min_w_0().child(control))
        };
        self.exclusions
            .update(cx, |state, _| state.set_editor_style(p.editor()));
        let state = self.exclusions.clone();
        let exclusions = InputBase::new("settings-exclusions")
            .h(px(95.))
            .w_full()
            .min_w_0()
            .p_2()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .text_size(self.scaled(13.))
            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                state.update(cx, |state, cx| state.focus(window, cx))
            })
            .child(Textarea::new(&self.exclusions));
        let mut fields = div()
            .id("settings-scroll")
            .test_support()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pr_2()
            .child(field(
                "setting-interval",
                self.text("Settings.Interval"),
                interval.into_any_element(),
            ))
            .child(field(
                "setting-low",
                self.text("Settings.Low"),
                low.into_any_element(),
            ))
            .child(field(
                "setting-growth",
                self.text("Settings.Growth"),
                growth.into_any_element(),
            ))
            .child(field(
                "setting-language",
                self.text("Settings.Language"),
                div()
                    .flex()
                    .gap_2()
                    .child(self.button(
                        "language-ru",
                        "Русский".into(),
                        Command::Setting(Setting::Language, "ru".into()),
                        true,
                        cx,
                    ))
                    .child(self.button(
                        "language-en",
                        "English".into(),
                        Command::Setting(Setting::Language, "en".into()),
                        true,
                        cx,
                    ))
                    .into_any_element(),
            ))
            .child(field(
                "setting-theme",
                self.text("Settings.Theme"),
                div()
                    .flex()
                    .gap_2()
                    .child(self.button(
                        "theme-light",
                        self.text("Theme.Light"),
                        Command::Setting(Setting::Theme, "light".into()),
                        true,
                        cx,
                    ))
                    .child(self.button(
                        "theme-dark",
                        self.text("Theme.Dark"),
                        Command::Setting(Setting::Theme, "dark".into()),
                        true,
                        cx,
                    ))
                    .child(self.button(
                        "theme-system",
                        self.text("Settings.Theme.System"),
                        Command::Setting(Setting::Theme, "system".into()),
                        true,
                        cx,
                    ))
                    .into_any_element(),
            ))
            .child(field("setting-scale", self.text("Display.Scale"), {
                let mut row = div().flex().flex_wrap().gap_1();
                for scale in [75, 90, 100, 110, 125, 150] {
                    row = row.child(self.button(
                        &format!("ui-scale-{scale}"),
                        format!("{scale}%"),
                        Command::Setting(Setting::UiScale, scale.to_string()),
                        true,
                        cx,
                    ));
                }
                row.into_any_element()
            }))
            .child(self.checkbox(
                "settings-battery",
                self.text("Settings.Battery"),
                view.settings.allow_on_battery,
                Command::Setting(
                    Setting::Battery,
                    (!view.settings.allow_on_battery).to_string(),
                ),
                true,
                cx,
            ))
            .child(self.checkbox(
                "settings-autostart",
                self.text("Settings.Autostart"),
                view.settings.autostart,
                Command::Setting(Setting::Autostart, (!view.settings.autostart).to_string()),
                true,
                cx,
            ))
            .child(field(
                "setting-exclusions",
                self.text("Settings.Exclusions"),
                exclusions.into_any_element(),
            ))
            .child(field(
                "setting-custom-temp",
                self.text("Settings.Temp"),
                custom.into_any_element(),
            ));
        fields = fields.child(self.note(self.text("Settings.Explanation")));
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .min_h_0()
            .child(self.title("Nav.Settings"))
            .child(fields)
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(p.border)
                    .pt_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.button(
                        "settings-save",
                        self.text("Action.Save"),
                        Command::SaveSettings,
                        !view.busy,
                        cx,
                    ))
                    .child(self.note(format!(
                        "{} · {}",
                        view.settings.language, view.settings.theme
                    ))),
            )
    }
    fn map(
        &mut self,
        view: &UiView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let search = self.edit("map-search", &self.search, window, cx);
        let mut crumbs = div()
            .id("map-crumbs-scroll")
            .test_support()
            .flex()
            .gap(px(4.))
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.crumbs_scroll);
        if view.map_path != self.crumbs_path {
            self.crumbs_path = view.map_path.clone();
            let n = view.map_parents.len() + view.map_breadcrumbs.len() * 2;
            if n > 0 {
                self.crumbs_scroll.scroll_to_item(n - 1);
            }
        }
        for (i, (path, label)) in view.map_parents.iter().enumerate() {
            crumbs = crumbs.child(
                self.button(
                    &format!("map-parent-{i}"),
                    label.clone(),
                    Command::MapWiden(path.clone()),
                    !view.busy,
                    cx,
                )
                .h(self.scaled(24.))
                .max_w(px(160.))
                .flex_shrink_0()
                .text_ellipsis_middle(),
            );
        }
        for (i, (index, label)) in view.map_breadcrumbs.iter().enumerate() {
            let index = *index;
            crumbs = crumbs
                .child(
                    self.button(
                        &format!("map-crumb-{i}"),
                        label.clone(),
                        Command::MapNavigate(index),
                        true,
                        cx,
                    )
                    .h(self.scaled(24.))
                    .max_w(px(160.))
                    .flex_shrink_0()
                    .text_ellipsis_middle(),
                )
                .child(
                    self.local_button(
                        &format!("map-siblings-{i}"),
                        "▾".into(),
                        move |this, _, cx| {
                            this.siblings_open = Some(index);
                            cx.notify();
                        },
                        cx,
                    )
                    .h(self.scaled(24.)),
                );
        }
        let mut controls = div()
            .id("map-controls")
            .test_support()
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .child(
                self.button("map-back", "←".into(), Command::MapBack, true, cx)
                    .accessibility_label(self.text("Map.Back")),
            )
            .child(
                self.button("map-forward", "→".into(), Command::MapForward, true, cx)
                    .accessibility_label(self.text("Map.Forward")),
            )
            .child(self.button("map-up", "↑".into(), Command::MapUp, true, cx))
            .child(
                self.button("map-root", "⌂".into(), Command::MapRoot, true, cx)
                    .accessibility_label(self.text("Map.Root")),
            )
            .child(self.local_button(
                "map-zoom-out",
                "−".into(),
                |this, _, cx| this.zoom_center(this.transform.scale / 1.3, cx),
                cx,
            ))
            .child(self.local_button(
                "map-zoom-in",
                "+".into(),
                |this, _, cx| this.zoom_center(this.transform.scale * 1.3, cx),
                cx,
            ))
            .child(self.local_button(
                "map-zoom-reset",
                "1:1".into(),
                |this, _, cx| {
                    this.transform = ViewTransform::default();
                    this.dispatch(Command::MapZoom(1.), cx);
                },
                cx,
            ))
            .child(self.local_button(
                "map-help",
                self.text("Display.Help"),
                |this, _, cx| {
                    this.keys_open = true;
                    cx.notify();
                },
                cx,
            ));
        for (metric, key) in [
            (0, "Map.Metric.Allocated"),
            (1, "Map.Metric.Logical"),
            (2, "Map.Metric.Files"),
        ] {
            controls = controls.child(self.button(
                &format!("map-metric-{metric}"),
                self.text(key),
                Command::MapMetric(metric),
                true,
                cx,
            ));
        }
        controls = controls
            .child(self.button(
                "map-age",
                self.text("Age.Mode"),
                Command::MapColor(1 - view.map_color),
                true,
                cx,
            ))
            .child(self.checkbox(
                "map-hidden",
                self.text("Display.Hidden"),
                view.settings.show_hidden,
                Command::Setting(
                    Setting::ShowHidden,
                    (!view.settings.show_hidden).to_string(),
                ),
                true,
                cx,
            ))
            .child(self.button(
                "map-depth-less",
                "[".into(),
                Command::Setting(
                    Setting::MapDepth,
                    view.settings.map_depth.saturating_sub(1).max(1).to_string(),
                ),
                true,
                cx,
            ))
            .child(self.note(format!(
                "{}: {}",
                self.text("Display.Depth"),
                view.settings.map_depth
            )))
            .child(self.button(
                "map-depth-more",
                "]".into(),
                Command::Setting(
                    Setting::MapDepth,
                    (view.settings.map_depth + 1).min(6).to_string(),
                ),
                true,
                cx,
            ));
        let bounds = self.map_bounds.get();
        let tiles = self.runtime.map_tiles(
            bounds.size.width.as_f32().max(180.),
            bounds.size.height.as_f32().max(180.),
        );
        *self.map_tiles.borrow_mut() = tiles.clone();
        let canvas = map_view::canvas_view(self, tiles, self.palette(), cx);
        let divider = div()
            .id("map-sidebar-divider")
            .test_support()
            .w(px(7.))
            .flex_shrink_0()
            .bg(self.palette().border)
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        this.dispatch(Command::Setting(Setting::SidebarWidth, "225".into()), cx);
                    } else {
                        // At 45% of the pane, the effective cap is .45/.55 * (canvas + divider).
                        let canvas = this.map_bounds.get().size.width.as_f32();
                        let effective_width = (this.runtime.view().settings.sidebar_width as f32)
                            .min((canvas + 7.) * 0.45 / 0.55);
                        this.sidebar_drag_start =
                            Some((event.position.x.as_f32(), effective_width));
                        this.sidebar_drag = true;
                    }
                }),
            );
        let mut marked = div()
            .id("map-marked-roots")
            .test_support()
            .flex()
            .flex_col()
            .gap_1()
            .min_h_0()
            .max_h(self.scaled(180.))
            .flex_shrink_0()
            .overflow_y_scroll();
        for (position, row) in view.map_marked.iter().take(2000).enumerate() {
            marked = marked.child(gpui_omarchy::with_tooltip(
                self.button(
                    &format!("map-marked-root-{position}"),
                    format!("{} · {}", self.text("Map.Unmark"), row.path),
                    Command::Mark(row.path.clone()),
                    !view.busy,
                    cx,
                )
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .text_ellipsis_middle(),
                row.path.clone(),
            ));
        }
        let mut recommendations = div()
            .id("map-recommendations-list")
            .test_support()
            .flex()
            .flex_col()
            .gap_1()
            .min_h_0()
            .max_h(self.scaled(180.))
            .flex_shrink_0()
            .overflow_y_scroll();
        for (position, row) in view.recommendations.iter().take(100).enumerate() {
            let Ok(index) = row.key.parse::<usize>() else {
                continue;
            };
            let label = format!("{} · {}", row.path, row.cells.join(" · "));
            recommendations = recommendations.child(gpui_omarchy::with_tooltip(
                self.button(
                    &format!("map-recommendation-{position}"),
                    label.clone(),
                    Command::MapNavigate(index),
                    !view.busy,
                    cx,
                )
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .text_ellipsis_middle(),
                label,
            ));
        }
        let sidebar = div()
            .id("map-sidebar")
            .test_support()
            .w(px(view.settings.sidebar_width as f32))
            .max_w(gpui_kit::relative(0.45))
            .flex_shrink_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_shrink_0()
                    .child(self.note(self.text("Map.Marked")))
                    .child(
                        div()
                            .id("map-marked-summary")
                            .test_support()
                            .text_size(self.scaled(12.))
                            .child(view.map_marked_summary.clone()),
                    )
                    .child(marked)
                    .child(
                        self.button(
                            "map-clear-marks",
                            self.text("Map.ClearMarks"),
                            Command::ClearMarks,
                            view.selected_count > 0 && !view.busy,
                            cx,
                        )
                        .flex_shrink_0(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_shrink_0()
                    .child(self.note(self.text("Map.Forecast")))
                    .child(
                        div()
                            .id("map-forecast-summary")
                            .test_support()
                            .min_h_0()
                            .max_h(px(150.))
                            .overflow_y_scroll()
                            .text_size(self.scaled(12.))
                            .child(view.map_forecast.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_shrink_0()
                    .child(
                        div()
                            .id("map-recommendations-title")
                            .test_support()
                            .text_size(self.scaled(12.))
                            .child(format!(
                                "{} · {}",
                                self.text("Map.Recommendations"),
                                view.recommendations.len()
                            )),
                    )
                    .child(self.note(self.text("Lists.Bound")))
                    .child(recommendations),
            )
            .child(self.note(view.map_visible_summary.clone()))
            .child(self.note(view.scan_reuse_notice.clone()))
            .when(self.details_open, |sidebar| {
                sidebar.child(
                    div()
                        .id("map-selection-details")
                        .test_support()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_shrink_0()
                        .child(gpui_omarchy::with_tooltip(
                            div()
                                .id("map-focused-path")
                                .text_ellipsis_middle()
                                .child(view.focused_path.clone()),
                            view.focused_path.clone(),
                        ))
                        .child(self.note(view.focused_summary.clone()))
                        .child(self.note(view.git_summary.clone()))
                        .child(self.map_legend()),
                )
            })
            .when(!view.map_covering_parent.is_empty(), |sidebar| {
                sidebar.child(
                    div()
                        .id("map-covered-parent")
                        .test_support()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_shrink_0()
                        .child(self.note(format!(
                            "{}\n{}",
                            self.text("Map.CoveredHint"),
                            view.map_covering_parent
                        )))
                        .child(self.button(
                            "map-unmark-parent",
                            self.text("Map.UnmarkParent"),
                            Command::Mark(view.map_covering_parent.clone()),
                            !view.busy,
                            cx,
                        )),
                )
            })
            .child(self.button(
                "map-mark-focus",
                self.text("Map.Mark"),
                Command::MapMark(self.active_tile().unwrap_or(0)),
                self.active_tile().is_some() && !view.busy && view.map_covering_parent.is_empty(),
                cx,
            ))
            .child(self.button(
                "map-review",
                self.text("Manual.Analyze"),
                Command::PreviewManual,
                view.can_manual && !view.busy,
                cx,
            ))
            .child(
                self.local_button(
                    "map-save-selected",
                    self.text("Selection.Save"),
                    |this, _, cx| this.export_marked(cx),
                    cx,
                )
                .disabled(view.selected_count == 0 || view.busy),
            )
            .child(
                self.local_button(
                    "map-copy-prompt",
                    self.text("Selection.Copy"),
                    |this, _, cx| this.copy_marked(cx),
                    cx,
                )
                .disabled(view.selected_count == 0),
            )
            .child(self.local_button(
                "sidebar-reset",
                self.text("Display.SidebarReset"),
                |this, _, cx| {
                    this.dispatch(Command::Setting(Setting::SidebarWidth, "225".into()), cx)
                },
                cx,
            ))
            .child(
                div()
                    .h(px(200.))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .child(self.table(
                        "map-object-list",
                        view.map_objects.clone(),
                        &["Logical"],
                        TableKind::Map,
                        cx,
                    )),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .size_full()
            .min_h_0()
            .child(controls)
            .child(crumbs)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(search))
                    .child(self.checkbox(
                        "map-isolate",
                        self.text("Map.Isolate"),
                        view.map_isolate,
                        Command::MapIsolate(!view.map_isolate),
                        true,
                        cx,
                    ))
                    .child(self.checkbox(
                        "map-global",
                        self.text("Map.Global"),
                        view.map_global,
                        Command::MapGlobal(!view.map_global),
                        true,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h(px(180.))
                            .child(canvas),
                    )
                    .child(divider)
                    .child(sidebar),
            )
    }
    pub fn zoom_center(&mut self, scale: f32, cx: &mut Context<Self>) {
        let bounds = self.map_bounds.get();
        self.transform.zoom_at(
            bounds.size.width.as_f32() / 2.,
            bounds.size.height.as_f32() / 2.,
            scale,
        );
        self.dispatch(Command::MapZoom(self.transform.scale), cx);
    }
    pub fn map_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.displayed_scan_id != self.runtime.scan_id() {
            self.reset_map_targets();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.modal_open() || self.editor_focused(window, cx) {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.control {
            return;
        }
        let target = self.active_tile();
        let command = match key {
            "enter" => target.map(Command::MapNavigate),
            "space" | "x" => target.map(Command::MapMark),
            "backspace" | "u" => Some(Command::MapUp),
            "escape" => {
                self.map_escape(window, cx);
                None
            }
            "left" if modifiers.alt => Some(Command::MapBack),
            "right" if modifiers.alt => Some(Command::MapForward),
            "left" | "right" | "up" | "down" | "h" | "j" | "k" | "l" => {
                self.keyboard_target = true;
                let direction = match key {
                    "left" | "h" => (-1., 0.),
                    "right" | "l" => (1., 0.),
                    "up" | "k" => (0., -1.),
                    _ => (0., 1.),
                };
                let tiles = self
                    .map_tiles
                    .borrow()
                    .iter()
                    .filter(|tile| tile.index != usize::MAX)
                    .map(|tile| map_view::HitRect {
                        index: tile.index,
                        x: tile.x,
                        y: tile.y,
                        width: tile.width,
                        height: tile.height,
                        depth: tile.depth,
                    })
                    .collect::<Vec<_>>();
                let next = target
                    .and_then(|i| map_view::neighbor(&tiles, i, direction))
                    .or_else(|| {
                        target
                            .is_none()
                            .then(|| tiles.first().map(|t| t.index))
                            .flatten()
                    });
                if let Some(i) = next {
                    self.focused_tile = Some(i);
                }
                next.map(Command::MapFocus)
            }
            "tab" => {
                self.keyboard_target = true;
                let siblings = target.map(|i| self.runtime.siblings(i)).unwrap_or_default();
                let current = target
                    .and_then(|i| siblings.iter().position(|r| r.key == i.to_string()))
                    .unwrap_or(0);
                let n = siblings.len();
                let next = if n == 0 {
                    None
                } else {
                    siblings[if modifiers.shift {
                        (current + n - 1) % n
                    } else {
                        (current + 1) % n
                    }]
                    .key
                    .parse::<usize>()
                    .ok()
                };
                if let Some(i) = next {
                    self.focused_tile = Some(i);
                }
                next.map(Command::MapFocus)
            }
            "+" | "=" => {
                self.zoom_center(self.transform.scale * 1.3, cx);
                None
            }
            "-" => {
                self.zoom_center(self.transform.scale / 1.3, cx);
                None
            }
            "0" => {
                self.transform = ViewTransform::default();
                Some(Command::MapZoom(1.))
            }
            "[" => Some(Command::Setting(
                Setting::MapDepth,
                self.runtime
                    .view()
                    .settings
                    .map_depth
                    .saturating_sub(1)
                    .max(1)
                    .to_string(),
            )),
            "]" => Some(Command::Setting(
                Setting::MapDepth,
                (self.runtime.view().settings.map_depth + 1)
                    .min(6)
                    .to_string(),
            )),
            "a" => Some(Command::MapColor(1 - self.runtime.view().map_color)),
            "i" => Some(Command::Setting(
                Setting::ShowHidden,
                (!self.runtime.view().settings.show_hidden).to_string(),
            )),
            "t" => {
                let view = self.runtime.view();
                let size_metric = view.map_size_metric;
                if view.map_color == 1 {
                    self.dispatch(Command::MapColor(0), cx);
                    Some(Command::MapMetric(size_metric))
                } else if view.map_metric == 2 {
                    self.dispatch(Command::MapMetric(size_metric), cx);
                    Some(Command::MapColor(1))
                } else {
                    Some(Command::MapMetric(2))
                }
            }
            "d" => Some(Command::MapSizeMetric(
                1 - self.runtime.view().map_size_metric,
            )),
            "r" | "f5" if !self.runtime.view().busy => Some(Command::Scan(false)),
            "g" if !self.runtime.view().busy => {
                self.dispatch(Command::SetRoot(crate::platform::system_root()), cx);
                Some(Command::Scan(false))
            }
            "v" => {
                self.volumes_open = true;
                window.focus(&self.dialog_focus, cx);
                cx.notify();
                None
            }
            "f" | "/" | "s" => {
                self.search.focus_handle(cx).focus(window, cx);
                None
            }
            "e" | "o" => target
                .and_then(|i| self.runtime.map_path(i))
                .map(Command::Open),
            "p" => {
                self.details_open = !self.details_open;
                cx.notify();
                None
            }
            "c" if self.runtime.view().can_manual && !self.runtime.view().busy => {
                Some(Command::PreviewManual)
            }
            "q" => Some(Command::Exit),
            "?" | "f1" => {
                self.keys_open = true;
                cx.notify();
                None
            }
            _ => return,
        };
        cx.stop_propagation();
        if let Some(command) = command {
            self.dispatch(command, cx);
        }
    }
    fn modal(
        &self,
        id: &str,
        title: String,
        content: impl IntoElement,
        footer: impl IntoElement,
    ) -> impl IntoElement {
        let p = self.palette();
        div()
            .id(id.to_owned())
            .role(gpui_kit::Role::Group)
            .test_support()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000099))
            .child(
                div()
                    .id(format!("{id}-popup"))
                    .focus_trap(format!("{id}-trap"), &self.dialog_focus)
                    .w(px(650.))
                    .max_w(gpui_kit::relative(0.94))
                    .max_h(gpui_kit::relative(0.88))
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .border_1()
                    .border_color(p.border)
                    .bg(p.panel)
                    .text_color(p.text)
                    .child(
                        div()
                            .text_size(self.scaled(19.))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(content)
                    .child(footer),
            )
    }
    fn overlays(&self, view: &UiView, cx: &Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let mut overlays = vec![];
        if let Some(review) = &view.review {
            let mut warnings = div()
                .id("review-warnings-scroll")
                .role(gpui_kit::Role::Group)
                .test_support()
                .aria_label(self.text("Cleanup.Warnings"))
                .flex()
                .flex_col()
                .gap_2()
                .max_h(px(112.))
                .min_h_0()
                .flex_shrink_0()
                .overflow_y_scroll();
            let mut details = div()
                .flex()
                .flex_col()
                .gap_2()
                .min_h_0()
                .flex_1()
                .child(self.note(review.summary.clone()));
            for (index, warning) in review.warnings.iter().enumerate() {
                warnings = warnings.child(
                    div()
                        .id(format!("review-warning-{index}"))
                        .role(gpui_kit::Role::Group)
                        .test_support()
                        .aria_label(warning.clone())
                        .flex_shrink_0()
                        .text_color(self.palette().danger)
                        .text_size(self.scaled(12.))
                        .child(warning.clone()),
                );
            }
            details = details
                .when(self.trash_notice, |details| {
                    details.child(
                        div()
                            .id("review-trash-unsupported")
                            .test_support()
                            .flex_shrink_0()
                            .text_color(self.palette().danger)
                            .text_size(self.scaled(12.))
                            .child(self.text("Keys.TrashUnsupported")),
                    )
                })
                .child(warnings)
                .child(
                    div()
                        .id("review-items-pane")
                        .role(gpui_kit::Role::Group)
                        .test_support()
                        .aria_label(self.text("Manual.Files"))
                        .min_h(px(100.))
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(self.table(
                            "review-items",
                            review.rows.clone(),
                            if review.cleanup {
                                &["Logical", "Category", "Reason"]
                            } else {
                                &["Logical"]
                            },
                            TableKind::Info,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .child(self.note(self.text("Manual.PermanentWarning"))),
                );
            let actions = div()
                .flex()
                .gap_2()
                .flex_shrink_0()
                .child(self.button(
                    "review-close",
                    self.text("Manual.Close"),
                    Command::DismissReview,
                    true,
                    cx,
                ))
                .child(
                    self.local_button(
                        "review-request-delete",
                        self.text("Manual.Delete"),
                        |this, window, cx| {
                            this.request_permanent_confirmation(window, cx);
                        },
                        cx,
                    )
                    .disabled(!review.can_confirm || view.busy),
                );
            overlays.push(
                self.modal("review-modal", review.title.clone(), details, actions)
                    .into_any_element(),
            );
            if self.confirmation.is_open() {
                let confirmation = div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.note(self.text(if review.cleanup {
                        "Cleanup.Confirm"
                    } else {
                        "Manual.Confirm"
                    })))
                    .child(self.note(review.summary.clone()));
                let actions = div()
                    .flex()
                    .gap_2()
                    .child(self.local_button(
                        "permanent-cancel",
                        self.text("Action.Cancel"),
                        |this, _, cx| {
                            this.confirmation.dismiss();
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(
                        self.local_button(
                            "permanent-confirm",
                            self.text("Manual.Delete"),
                            |this, _, cx| {
                                if let Some(review) = this.runtime.view().review.clone()
                                    && this.confirmation.take(
                                        &review.id,
                                        review.cleanup,
                                        review.can_confirm,
                                    )
                                {
                                    this.dispatch(
                                        if review.cleanup {
                                            Command::ConfirmCleanup
                                        } else {
                                            Command::ConfirmManual
                                        },
                                        cx,
                                    );
                                }
                            },
                            cx,
                        )
                        .disabled(view.busy),
                    );
                overlays.push(
                    self.modal(
                        "permanent-dialog",
                        self.text("Cleanup.Warning"),
                        confirmation,
                        actions,
                    )
                    .into_any_element(),
                );
            }
        }
        if self.export_disclosure {
            let actions = div()
                .flex()
                .gap_2()
                .child(self.local_button(
                    "export-cancel",
                    self.text("Action.Cancel"),
                    |this, _, cx| {
                        this.export_disclosure = false;
                        cx.notify();
                    },
                    cx,
                ))
                .child(self.local_button(
                    "export-continue",
                    self.text("Action.Export"),
                    |this, _, cx| this.export(cx),
                    cx,
                ));
            overlays.push(
                self.modal(
                    "export-disclosure",
                    self.text("Action.Export"),
                    self.note(self.text("Export.Disclosure")),
                    actions,
                )
                .into_any_element(),
            );
        }
        if self.volumes_open {
            let mut rows = div()
                .id("drive-list")
                .flex()
                .flex_col()
                .gap_2()
                .max_h(px(250.))
                .overflow_y_scroll();
            let mut drives: Vec<_> = view
                .drives
                .iter()
                .map(|(root, label)| (root, label, crate::platform::volume_space(root).ok()))
                .collect();
            let fraction = |space: &Option<diskburrow_services::VolumeSpace>| {
                space
                    .as_ref()
                    .filter(|s| s.total_bytes > 0)
                    .map_or(f64::INFINITY, |s| {
                        s.free_bytes as f64 / s.total_bytes as f64
                    })
            };
            drives.sort_by(|a, b| fraction(&a.2).total_cmp(&fraction(&b.2)));
            for (i, (root, label, space)) in drives.into_iter().enumerate() {
                let root = root.clone();
                let label = space.map_or_else(
                    || format!("{label} · {}", self.text("Unknown")),
                    |s| {
                        format!(
                            "{label} · {:.1}% · {}: {} / {}",
                            100. * (1. - s.free_bytes as f64 / s.total_bytes.max(1) as f64),
                            self.text("Map.VolumeFree"),
                            crate::locale::gb(s.free_bytes, &view.settings.language),
                            crate::locale::gb(s.total_bytes, &view.settings.language)
                        )
                    },
                );
                rows = rows.child(self.local_button(
                    &format!("drive-{i}"),
                    label,
                    move |this, _, cx| {
                        this.volumes_open = false;
                        this.dispatch(Command::SetRoot(root.clone()), cx);
                    },
                    cx,
                ));
            }
            overlays.push(
                self.modal(
                    "volumes-dialog",
                    self.text("Map.Volumes"),
                    rows,
                    self.local_button(
                        "drive-close",
                        self.text("Action.Cancel"),
                        |this, _, cx| {
                            this.volumes_open = false;
                            cx.notify();
                        },
                        cx,
                    ),
                )
                .into_any_element(),
            );
        }
        if self.keys_open {
            let content = div()
                .id("keys-content")
                .flex()
                .flex_col()
                .min_h_0()
                .overflow_y_scroll()
                .text_size(self.scaled(12.))
                .child(self.text("Display.Keys"));
            overlays.push(
                self.modal(
                    "keys-dialog",
                    self.text("Display.Help"),
                    content,
                    self.local_button(
                        "keys-close",
                        self.text("Action.Cancel"),
                        |this, _, cx| {
                            this.keys_open = false;
                            cx.notify();
                        },
                        cx,
                    ),
                )
                .into_any_element(),
            );
        }
        if let Some(target) = self.siblings_open {
            let mut rows = div()
                .id("siblings-scroll")
                .flex()
                .flex_col()
                .gap_1()
                .min_h_0()
                .overflow_y_scroll();
            for (i, row) in self.runtime.siblings(target).into_iter().enumerate() {
                let Ok(index) = row.key.parse::<usize>() else {
                    continue;
                };
                let label = format!("{} · {}", row.path, row.cells.join(" · "));
                rows = rows.child(
                    self.local_button(
                        &format!("sibling-{i}"),
                        label,
                        move |this, _, cx| {
                            this.siblings_open = None;
                            this.dispatch(Command::MapNavigate(index), cx);
                            this.dispatch(Command::MapFocus(index), cx);
                        },
                        cx,
                    )
                    .min_w_0()
                    .text_ellipsis_middle(),
                );
            }
            overlays.push(
                self.modal(
                    "siblings-dialog",
                    self.text("Display.Siblings"),
                    rows,
                    self.local_button(
                        "siblings-close",
                        self.text("Action.Cancel"),
                        |this, _, cx| {
                            this.siblings_open = None;
                            cx.notify();
                        },
                        cx,
                    ),
                )
                .into_any_element(),
            );
        }
        overlays
    }
}
impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.displayed_scan_id != self.runtime.scan_id() {
            self.reset_map_targets();
            self.displayed_scan_id = self.runtime.scan_id();
        }
        let view = self.runtime.view().clone();
        window.set_rem_size(px(
            self.base_rem * view.settings.ui_scale_percent as f32 / 100.
        ));
        if view.root != self.last_root {
            self.last_root = view.root.clone();
            self.root.update(cx, |state, cx| {
                state.set_value(view.root.clone(), window, cx)
            });
        }
        let p = self.palette();
        let top = self.top(&view, window, cx);
        let nav = self.navigation(cx);
        let body = match self.page {
            Page::Overview => self.overview(&view, cx).into_any_element(),
            Page::Largest => self.largest(&view, cx).into_any_element(),
            Page::Map => self.map(&view, window, cx).into_any_element(),
            Page::History => self.history(&view, cx).into_any_element(),
            Page::Cleanup => self.cleanup(&view, window, cx).into_any_element(),
            Page::Settings => self.settings(&view, window, cx).into_any_element(),
        };
        let mut root = div()
            .id("diskburrow-app")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .font_family("Segoe UI")
            .text_size(self.scaled(13.))
            .bg(p.background)
            .text_color(p.text)
            .child(top)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(nav)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .p(px(8.))
                            .overflow_hidden()
                            .child(body),
                    ),
            )
            .child(
                div()
                    .id("status-bar")
                    .role(gpui_kit::Role::Status)
                    .test_support()
                    .aria_label(view.status.clone())
                    .flex_shrink_0()
                    .min_h(px(28.))
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(p.border)
                    .text_size(self.scaled(12.))
                    .child(view.status.clone()),
            )
            .on_mouse_move(
                cx.listener(|this, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    if this.modal_open() {
                        return;
                    }
                    if this.sidebar_drag
                        && event.pressed_button == Some(gpui_kit::MouseButton::Left)
                    {
                        let Some((start_x, start_width)) = this.sidebar_drag_start else {
                            return;
                        };
                        // Apply pointer displacement to the rendered width, independent of where
                        // in the divider the drag started or whether the saved width was capped.
                        let width = (start_width + start_x - event.position.x.as_f32())
                            .round()
                            .clamp(180., 420.) as u16;
                        if width != this.runtime.view().settings.sidebar_width {
                            this.dispatch(
                                Command::Setting(Setting::SidebarWidth, width.to_string()),
                                cx,
                            );
                        }
                    } else {
                        this.sidebar_drag = false;
                        this.sidebar_drag_start = None;
                    }
                }),
            )
            .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.sidebar_drag = false;
                    this.sidebar_drag_start = None;
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.global_key(event, window, cx)
            }));
        if let Some(error) = view.error.clone() {
            root = root.child(
                div()
                    .id("error-message")
                    .absolute()
                    .bottom(px(29.))
                    .left(px(155.))
                    .right_0()
                    .max_h(px(90.))
                    .p_2()
                    .bg(p.panel)
                    .text_color(p.danger)
                    .text_size(self.scaled(12.))
                    .child(error),
            );
        }
        for overlay in self.overlays(&view, cx) {
            root = root.child(overlay);
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, size};
    #[gpui_kit::test]
    fn port_map_exposes_remaining_controls_in_minimum_window(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        cx.update(gpui_omarchy::init);
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            App::new(
                Runtime::new(fixture.path().join("data")).unwrap(),
                window,
                cx,
            )
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-map", cx);
            assert!(window.find("map-help").visible());
            assert!(window.find("map-age").visible());
            assert!(window.find("map-hidden").visible());
            assert!(window.find("map-depth-more").visible());
            window.click("map-help", cx);
            assert!(window.find("keys-dialog").visible());
            window.click("keys-close", cx);
            assert!(window.find("map-viewport").bounds().size.height >= px(180.));
        })
        .unwrap();
    }
    #[gpui_kit::test]
    fn long_eight_root_review_keeps_rows_and_confirmation_actions_visible(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("long-reviewed-root-name-".repeat(4));
        std::fs::create_dir(&root).unwrap();
        let mut paths = vec![];
        for i in 0..8 {
            let file = root.join(format!("{i}-{}.txt", "long-reviewed-file-name-".repeat(6)));
            std::fs::write(&file, b"owned review fixture").unwrap();
            paths.push(file);
        }
        let idle = |runtime: &mut Runtime| {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while runtime.view().busy {
                runtime.poll();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        cx.update(gpui_omarchy::init);
        for language in ["ru", "en"] {
            for theme in ["light", "dark"] {
                let mut runtime =
                    Runtime::new(fixture.path().join(format!("data-{language}-{theme}"))).unwrap();
                idle(&mut runtime);
                runtime.command(Command::Setting(Setting::Language, language.into()));
                runtime.command(Command::Setting(Setting::Theme, theme.into()));
                runtime.command(Command::SetRoot(root.to_string_lossy().into_owned()));
                runtime.command(Command::Scan(false));
                idle(&mut runtime);
                for file in &paths {
                    runtime.command(Command::Mark(file.to_string_lossy().into_owned()));
                }
                runtime.command(Command::PreviewManual);
                idle(&mut runtime);
                assert!(runtime.view().review.as_ref().unwrap().can_confirm);
                assert!(runtime.view().review.as_ref().unwrap().warnings.len() >= 11);
                let last_warning = format!(
                    "review-warning-{}",
                    runtime.view().review.as_ref().unwrap().warnings.len() - 1
                );
                let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
                    App::new(runtime, window, cx)
                });
                cx.update_window(handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    assert!(window.find("review-items-0").visible());
                    assert!(window.find("review-items-pane").bounds().size.height >= px(100.));
                    assert!(window.find("review-warnings-scroll").bounds().size.height <= px(112.));
                    window.scroll(
                        "review-warnings-scroll",
                        gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-10000.))),
                        cx,
                    );
                    assert!(window.find(last_warning.clone()).visible());
                    assert!(window.find("review-close").visible());
                    assert!(window.find("review-request-delete").visible());
                    assert!(window.find("review-request-delete").bounds().bottom() <= px(600.));
                    window.click("review-request-delete", cx);
                    assert!(window.find("permanent-cancel").visible());
                    assert!(window.find("permanent-confirm").bounds().bottom() <= px(600.));
                    window.click("permanent-cancel", cx);
                    window.click("review-close", cx);
                })
                .unwrap();
            }
        }
        assert!(paths.iter().all(|file| file.exists()));
    }

    #[gpui_kit::test]
    fn recommendation_click_only_navigates_and_keeps_fixture_unmarked(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("scan");
        let cache = root.join(".cache");
        std::fs::create_dir_all(&cache).unwrap();
        let file = cache.join("large-owned-fixture.bin");
        std::fs::File::create(&file)
            .unwrap()
            .set_len(72 * 1024 * 1024)
            .unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        let idle = |runtime: &mut Runtime| {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while runtime.view().busy {
                runtime.poll();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        idle(&mut runtime);
        runtime.command(Command::SetRoot(root.to_string_lossy().into_owned()));
        runtime.command(Command::Setting(Setting::Language, "en".into()));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        assert_eq!(runtime.view().recommendations.len(), 1);
        assert_eq!(runtime.view().recommendations[0].cells[0], "Cache");
        assert_eq!(
            runtime.view().recommendations[0].cells[1],
            "Regenerable cache"
        );
        cx.update(gpui_omarchy::init);
        let mut entity = None;
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            entity = Some(cx.entity());
            App::new(runtime, window, cx)
        });
        let app = entity.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-cleanup", cx);
            window.click("cleanup-tab-recommendations", cx);
            window.click("insights-list-path-0", cx);
            assert!(window.find("map-viewport").visible());
        })
        .unwrap();
        app.update(cx, |app, _| {
            assert_eq!(app.page, Page::Map);
            assert_eq!(app.runtime.view().selected_count, 0);
            assert!(app.runtime.view().review.is_none());
            assert!(
                app.runtime
                    .view()
                    .focused_path
                    .eq_ignore_ascii_case(&cache.to_string_lossy())
            );
            assert!(app.runtime.view().focused_summary.contains("Cache"));
        });
        assert!(file.exists());
    }

    #[gpui_kit::test]
    fn insights_tab_is_localized_and_visible_in_minimum_window(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        cx.update(gpui_omarchy::init);
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            App::new(Runtime::new(fixture.path().to_owned()).unwrap(), window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-settings", cx);
            window.click("language-en", cx);
            window.click("nav-cleanup", cx);
            window.click("cleanup-tab-recommendations", cx);
            assert_eq!(
                window.find("insights-heading").label(),
                Some("Worth a look")
            );
            assert!(window.find("insights-heading").visible());
            window.click("nav-settings", cx);
            window.click("theme-dark", cx);
            window.click("language-ru", cx);
            window.click("nav-cleanup", cx);
            assert_eq!(
                window.find("insights-heading").label(),
                Some("Стоит посмотреть")
            );
            assert!(window.find("insights-heading").bounds().bottom() <= px(572.));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn minimum_window_keeps_long_path_map_and_review_visible(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture
            .path()
            .join("long-root-name-for-map-layout-check".repeat(4));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("largest.bin"), vec![0; 10000]).unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        let idle = |runtime: &mut Runtime| {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while runtime.view().busy {
                runtime.poll();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        idle(&mut runtime);
        runtime.command(Command::SetRoot(root.to_string_lossy().into_owned()));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        assert!(runtime.view().error.is_none());
        cx.update(gpui_omarchy::init);
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            App::new(runtime, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-map", cx);
            let canvas = window.find("map-viewport");
            assert!(canvas.visible());
            assert!(canvas.bounds().size.height >= px(180.));
            assert!(canvas.bounds().bottom() <= px(572.));
            assert_eq!(
                window.find("map-metric-0").label(),
                Some(crate::locale::text("ru", "Map.Metric.Allocated").as_str())
            );
            window.click("nav-largest", cx);
            assert!(window.find("manual-review").visible());
            window.click("nav-settings", cx);
            assert!(window.find("settings-save").visible());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn settings_native_fields_edit_and_save_stays_inside_minimum_window(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        cx.update(gpui_omarchy::init);
        let mut entity = None;
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            entity = Some(cx.entity());
            App::new(Runtime::new(fixture.path().to_owned()).unwrap(), window, cx)
        });
        let app = entity.unwrap();
        app.update(cx, |app, cx| {
            app.page = Page::Settings;
            cx.notify();
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let save = window.find("settings-save");
            assert!(save.visible());
            assert!(save.bounds().bottom() <= px(600.));
            assert!(window.find("settings-low").bounds().size.width > px(100.));
            window.click("settings-low", cx);
            window.press("ctrl-a", cx);
            window.input("15.000000001", cx);
        })
        .unwrap();
        app.update(cx, |app, cx| {
            assert_eq!(app.low.read(cx).value(), "15.000000001")
        });
    }

    #[gpui_kit::test]
    fn page_navigation_and_live_language_theme_change_rendered_controls(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        cx.update(gpui_omarchy::init);
        let handle = cx.open_window(size(px(880.), px(600.)), |window, cx| {
            App::new(Runtime::new(fixture.path().to_owned()).unwrap(), window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            for id in [
                "nav-largest",
                "nav-map",
                "nav-history",
                "nav-cleanup",
                "nav-settings",
                "nav-overview",
            ] {
                window.click(id, cx);
            }
            window.click("nav-settings", cx);
            window.click("language-en", cx);
            assert_eq!(window.find("settings-save").label(), Some("Save settings"));
            window.click("theme-dark", cx);
            assert!(window.find("settings-save").visible());
            window.click("language-ru", cx);
            assert_ne!(window.find("settings-save").label(), Some("Save settings"));
        })
        .unwrap();
    }
}

#[cfg(test)]
#[path = "ui_port_tests.rs"]
mod port_tests;
