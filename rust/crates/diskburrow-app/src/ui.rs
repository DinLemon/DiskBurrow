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
    pub fn new(runtime: Runtime, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
                        let changed = this.runtime.poll();
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
            runtime,
            page: Page::Overview,
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
            dialog_focus: cx.focus_handle(),
            confirmation: ConfirmationGate::default(),
            export_disclosure: false,
            volumes_open: false,
            largest_files: false,
            history_changes: false,
            cleanup_tab: 0,
        }
    }
    pub fn dispatch(&mut self, command: Command, cx: &mut Context<Self>) {
        self.confirmation.dismiss();
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
        ) {
            self.transform = ViewTransform::default();
            self.focused_tile = None;
        }
        self.runtime.command(command);
        cx.notify();
    }
    fn text(&self, key: &str) -> String {
        crate::locale::text(&self.runtime.view().settings.language, key)
    }
    fn palette(&self) -> Palette {
        Palette::new(&self.runtime.view().settings.theme)
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
            .px_3()
            .h(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .text_color(if enabled { p.text } else { p.muted })
            .text_size(px(12.))
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
            .px_3()
            .h(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .text_color(p.text)
            .text_size(px(12.))
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
            .text_size(px(12.))
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
            .h(px(33.))
            .min_w_0()
            .w_full()
            .px_2()
            .border_1()
            .border_color(p.border)
            .bg(p.panel)
            .text_color(p.text)
            .text_size(px(13.))
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
            .text_size(px(12.))
            .text_color(self.palette().muted)
            .child(text)
    }
    fn title(&self, key: &str) -> Div {
        div()
            .text_size(px(20.))
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
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_2()
            .p_3()
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
                            .text_size(px(19.))
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
                    .gap_2()
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
                    .text_size(px(12.))
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
            .flex()
            .gap_2()
            .items_center()
            .h(px(30.))
            .flex_shrink_0()
            .px_2()
            .bg(p.inset)
            .text_size(px(11.))
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
                            .h(px(34.))
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
                            .text_size(px(12.))
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
        let mut cards = div().flex().flex_wrap().gap_3();
        for (index, (key, value)) in view.overview.iter().enumerate() {
            cards = cards.child(
                self.panel(&format!("overview-{index}"))
                    .min_w(px(165.))
                    .flex_1()
                    .child(self.note(crate::locale::text(&view.settings.language, key)))
                    .child(gpui_omarchy::with_tooltip(
                        div()
                            .id(format!("overview-value-{index}"))
                            .text_size(px(22.))
                            .text_ellipsis()
                            .child(value.clone()),
                        value.clone(),
                    )),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .min_h_0()
            .child(self.title("Nav.Overview"))
            .child(cards)
            .child(gpui_omarchy::with_tooltip(
                div()
                    .id("volume-stamp")
                    .h(px(18.))
                    .text_size(px(11.))
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
            .text_size(px(10.));
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
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(legend)
            .child(gpui_omarchy::with_tooltip(
                div()
                    .id("map-hatch-note")
                    .text_size(px(10.))
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
        let mut categories = div().flex().flex_wrap().gap_1();
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
                .gap_2()
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
                        .text_size(px(11.))
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
                let mut buttons = div().flex().flex_wrap().gap_1();
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
                    .gap_2()
                    .flex_1()
                    .min_h_0()
                    .child(self.note(self.text("Recommend.Text")))
                    .child(buttons)
                    .child(gpui_omarchy::with_tooltip(
                        div()
                            .id("observed-cache-limit")
                            .h(px(18.))
                            .text_size(px(11.))
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
            .gap_2()
            .size_full()
            .min_h_0()
            .child(self.title("Nav.Cleanup"))
            .child(gpui_omarchy::with_tooltip(
                div()
                    .id("cleanup-scope")
                    .h(px(18.))
                    .text_size(px(11.))
                    .text_ellipsis()
                    .child(self.text("Cleanup.Scope")),
                self.text("Cleanup.Scope"),
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
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
                    .gap_2()
                    .items_center()
                    .child(div().w(px(100.)).child(self.text("Filter")))
                    .child(div().flex_1().child(filter)),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
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
                        .gap_1()
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
                    .text_size(px(11.))
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
                        .text_size(px(12.))
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
            .text_size(px(13.))
            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                state.update(cx, |state, cx| state.focus(window, cx))
            })
            .child(Textarea::new(&self.exclusions));
        let mut fields = div()
            .id("settings-scroll")
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
                    .into_any_element(),
            ))
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
        &self,
        view: &UiView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let search = self.edit("map-search", &self.search, window, cx);
        let mut crumbs = div().flex().gap_1().min_w_0().overflow_hidden();
        for (i, (index, label)) in view.map_breadcrumbs.iter().enumerate() {
            crumbs = crumbs.child(self.button(
                &format!("map-crumb-{i}"),
                label.clone(),
                Command::MapNavigate(*index),
                true,
                cx,
            ));
        }
        let controls = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(self.button(
                "map-back",
                self.text("Map.Back"),
                Command::MapBack,
                true,
                cx,
            ))
            .child(self.button(
                "map-forward",
                self.text("Map.Forward"),
                Command::MapForward,
                true,
                cx,
            ))
            .child(self.button("map-up", self.text("Map.Up"), Command::MapUp, true, cx))
            .child(self.button(
                "map-root",
                self.text("Map.Root"),
                Command::MapRoot,
                true,
                cx,
            ))
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
                self.text("Map.Reset"),
                |this, _, cx| {
                    this.transform = ViewTransform::default();
                    this.dispatch(Command::MapZoom(1.), cx);
                },
                cx,
            ));
        let mut metrics = div().flex().gap_1();
        for (metric, key) in [
            (0, "Map.Metric.Allocated"),
            (1, "Map.Metric.Logical"),
            (2, "Map.Metric.Files"),
        ] {
            metrics = metrics.child(self.button(
                &format!("map-metric-{metric}"),
                self.text(key),
                Command::MapMetric(metric),
                true,
                cx,
            ));
        }
        let bounds = self.map_bounds.get();
        let tiles = self.runtime.map_tiles(
            bounds.size.width.as_f32().max(180.),
            bounds.size.height.as_f32().max(180.),
        );
        *self.map_tiles.borrow_mut() = tiles.clone();
        let canvas = map_view::canvas_view(self, tiles, self.palette(), cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .size_full()
            .min_h_0()
            .child(self.title("Nav.Map"))
            .child(controls)
            .child(crumbs)
            .child(metrics)
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
                    .gap_3()
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
                    .child(
                        div()
                            .w(px(225.))
                            .flex_shrink_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(self.note(format!(
                                "{}: {}",
                                self.text("Map.Matches"),
                                view.map_matches
                            )))
                            .child(gpui_omarchy::with_tooltip(
                                div()
                                    .id("map-focused-path")
                                    .text_ellipsis_middle()
                                    .child(view.focused_path.clone()),
                                view.focused_path.clone(),
                            ))
                            .child(self.note(view.focused_summary.clone()))
                            .child(gpui_omarchy::with_tooltip(
                                div()
                                    .id("git-detail")
                                    .max_h(px(72.))
                                    .text_size(px(11.))
                                    .overflow_hidden()
                                    .child(view.git_summary.clone()),
                                view.git_summary.clone(),
                            ))
                            .child(self.map_legend())
                            .child(self.button(
                                "map-mark-focus",
                                self.text("Map.Mark"),
                                Command::MapMark(self.focused_tile.unwrap_or(0)),
                                self.focused_tile.is_some(),
                                cx,
                            ))
                            .child(self.button(
                                "map-review",
                                self.text("Manual.Analyze"),
                                Command::PreviewManual,
                                view.can_manual && !view.busy,
                                cx,
                            ))
                            .child(self.table(
                                "map-object-list",
                                view.map_objects.clone(),
                                &["Logical"],
                                TableKind::Map,
                                cx,
                            )),
                    ),
            )
            .child(gpui_omarchy::with_tooltip(
                div()
                    .id("map-help")
                    .h(px(20.))
                    .text_size(px(11.))
                    .text_ellipsis()
                    .text_color(self.palette().muted)
                    .child(self.text("Map.Legend")),
                format!(
                    "{}\n{}",
                    self.text("Map.AllocationNote"),
                    self.text("Map.Keys")
                ),
            ))
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
    pub fn map_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let command = match key {
            "enter" => self.focused_tile.map(Command::MapNavigate),
            "space" => self.focused_tile.map(Command::MapMark),
            "backspace" | "escape" => Some(Command::MapUp),
            "left" if event.keystroke.modifiers.alt => Some(Command::MapBack),
            "right" if event.keystroke.modifiers.alt => Some(Command::MapForward),
            "left" | "right" | "up" | "down" => {
                let direction = match key {
                    "left" => (-1., 0.),
                    "right" => (1., 0.),
                    "up" => (0., -1.),
                    _ => (0., 1.),
                };
                let tiles = self
                    .map_tiles
                    .borrow()
                    .iter()
                    .map(|tile| map_view::HitRect {
                        index: tile.index,
                        x: tile.x,
                        y: tile.y,
                        width: tile.width,
                        height: tile.height,
                        depth: tile.depth,
                    })
                    .collect::<Vec<_>>();
                let next = self
                    .focused_tile
                    .and_then(|index| map_view::neighbor(&tiles, index, direction))
                    .or_else(|| {
                        self.focused_tile
                            .is_none()
                            .then(|| tiles.first().map(|tile| tile.index))
                            .flatten()
                    });
                if let Some(index) = next {
                    self.focused_tile = Some(index);
                    Some(Command::MapFocus(index))
                } else {
                    None
                }
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
            _ => None,
        };
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
    ) -> Stateful<Div> {
        let p = self.palette();
        div()
            .id(id.to_owned())
            .absolute()
            .inset_0()
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
                            .text_size(px(19.))
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
            let mut details = div()
                .flex()
                .flex_col()
                .gap_2()
                .min_h_0()
                .flex_1()
                .child(self.note(review.summary.clone()));
            for warning in &review.warnings {
                details = details.child(
                    div()
                        .text_color(self.palette().danger)
                        .text_size(px(12.))
                        .child(warning.clone()),
                );
            }
            details = details
                .child(
                    div()
                        .h(px(210.))
                        .min_h_0()
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
                .child(self.note(self.text("Manual.PermanentWarning")));
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
                            if let Some(review) = this.runtime.view().review.clone() {
                                this.confirmation.request(
                                    &review.id,
                                    review.cleanup,
                                    review.can_confirm,
                                );
                                window.focus(&this.dialog_focus, cx);
                                cx.notify();
                            }
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
        overlays
    }
}
impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.runtime.view().clone();
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
            .text_size(px(13.))
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
                            .p_3()
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
                    .text_size(px(12.))
                    .child(view.status.clone()),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    if this.confirmation.is_open() {
                        this.confirmation.dismiss();
                    } else if this.export_disclosure {
                        this.export_disclosure = false;
                    } else if this.volumes_open {
                        this.volumes_open = false;
                    } else if this.runtime.view().review.is_some() {
                        this.dispatch(Command::DismissReview, cx);
                    }
                    cx.notify();
                }
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
                    .text_size(px(12.))
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
