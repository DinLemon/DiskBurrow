//! Canvas geometry and pointer transform; layout belongs to the full live index.
#[derive(Clone, Copy, Debug)]
pub struct HitRect {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub depth: u32,
}
pub fn hit_test(tiles: &[HitRect], x: f32, y: f32) -> Option<usize> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    tiles
        .iter()
        .filter(|t| {
            t.width > 0.
                && t.height > 0.
                && x >= t.x
                && y >= t.y
                && x < t.x + t.width
                && y < t.y + t.height
        })
        .max_by_key(|t| t.depth)
        .map(|t| t.index)
}
pub fn neighbor(tiles: &[HitRect], index: usize, direction: (f32, f32)) -> Option<usize> {
    let origin = tiles.iter().find(|tile| tile.index == index)?;
    let center = (origin.x + origin.width / 2., origin.y + origin.height / 2.);
    tiles
        .iter()
        .filter(|tile| tile.index != index && tile.width > 0. && tile.height > 0.)
        .filter_map(|tile| {
            let dx = tile.x + tile.width / 2. - center.0;
            let dy = tile.y + tile.height / 2. - center.1;
            let along = dx * direction.0 + dy * direction.1;
            let across = dx * direction.1 - dy * direction.0;
            (along > 0.).then_some((tile.index, along * along + across * across * 2.))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(index, _)| index)
}
#[derive(Clone, Copy, Debug)]
pub struct ViewTransform {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
}
impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            scale: 1.,
            x: 0.,
            y: 0.,
        }
    }
}
impl ViewTransform {
    pub fn base(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.scale + self.x, y / self.scale + self.y)
    }
    pub fn zoom_at(&mut self, x: f32, y: f32, scale: f32) {
        if !scale.is_finite() || !x.is_finite() || !y.is_finite() {
            return;
        }
        let anchor = self.base(x, y);
        self.scale = scale.clamp(1., 16.);
        self.x = anchor.0 - x / self.scale;
        self.y = anchor.1 - y / self.scale;
    }
    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.x -= dx / self.scale;
        self.y -= dy / self.scale;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelAction {
    Ignore,
    Pan { dy: f32 },
    Zoom { scale: f32 },
    Navigate(usize),
    Up,
}

pub fn wheel_action(
    transform: ViewTransform,
    lines: f32,
    shift: bool,
    viewport: (f32, f32),
    pointer: (f32, f32),
    tiles: &[(HitRect, bool)],
) -> WheelAction {
    if !lines.is_finite()
        || lines.abs() < f32::EPSILON
        || !transform.scale.is_finite()
        || transform.scale <= 0.
        || !transform.x.is_finite()
        || !transform.y.is_finite()
        || !viewport.0.is_finite()
        || !viewport.1.is_finite()
        || viewport.0 <= 0.
        || viewport.1 <= 0.
        || !pointer.0.is_finite()
        || !pointer.1.is_finite()
        || pointer.0 < 0.
        || pointer.1 < 0.
        || pointer.0 >= viewport.0
        || pointer.1 >= viewport.1
    {
        return WheelAction::Ignore;
    }
    if shift {
        let dy = lines * 30.;
        return if dy.is_finite() {
            WheelAction::Pan { dy }
        } else {
            WheelAction::Ignore
        };
    }
    if lines < 0. && transform.scale <= 1. {
        return WheelAction::Up;
    }
    if lines > 0. {
        // Inspect the current transform, so the event that fills a directory only zooms.
        let filled = tiles
            .iter()
            .filter(|(tile, directory)| {
                let left = (tile.x - transform.x) * transform.scale;
                let top = (tile.y - transform.y) * transform.scale;
                let right = left + tile.width * transform.scale;
                let bottom = top + tile.height * transform.scale;
                *directory
                    && tile.index != usize::MAX
                    && tile.width > 0.
                    && tile.height > 0.
                    && [left, top, right, bottom]
                        .iter()
                        .all(|value| value.is_finite())
                    && left <= 0.5
                    && top <= 0.5
                    && right >= viewport.0 - 0.5
                    && bottom >= viewport.1 - 0.5
            })
            .max_by_key(|(tile, _)| tile.depth);
        if let Some((tile, _)) = filled {
            return WheelAction::Navigate(tile.index);
        }
    }
    let scale = (transform.scale * if lines > 0. { 1.15 } else { 1. / 1.15 }).clamp(1., 16.);
    if scale == transform.scale {
        WheelAction::Ignore
    } else {
        WheelAction::Zoom { scale }
    }
}

fn label_font_size(percent: u16) -> f32 {
    11. * f32::from(percent) / 100.
}

fn age_rgb(age: u8) -> u32 {
    match age {
        1 => 0x397c70,
        2 => 0x487caf,
        3 => 0x8a8545,
        4 => 0xac733e,
        5 => 0x8f4b62,
        _ => 0x697784,
    }
}

use crate::{
    contract::{Command, MapTile},
    ui::{App, Palette},
};
use gpui_kit::{
    BorderStyle, Bounds, ContentMask, Context, Corners, Edges, Font, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, NavigationDirection, ParentElement as _, PathBuilder, Point,
    ScrollDelta, ScrollWheelEvent, SharedString, Size, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, TextAlign, TextRun, canvas, div, px, quad,
};

pub fn canvas_view(
    app: &App,
    tiles: Vec<MapTile>,
    palette: Palette,
    cx: &Context<App>,
) -> impl IntoElement {
    let measured = app.map_bounds.clone();
    let transform = app.transform;
    let focused = app.active_tile();
    let color_mode = app.runtime.view().map_color;
    let interface_scale = f32::from(app.runtime.view().settings.ui_scale_percent) / 100.;
    let font_size = label_font_size(app.runtime.view().settings.ui_scale_percent);
    div()
        .id("map-viewport")
        .role(gpui_kit::Role::Group)
        .test_support()
        .aria_label(crate::locale::text(
            &app.runtime.view().settings.language,
            "Nav.Map",
        ))
        .track_focus(&app.map_focus)
        .relative()
        .flex_1()
        .min_h(px(180.))
        .min_w_0()
        .overflow_hidden()
        .bg(palette.inset)
        .on_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
            window.focus(&this.map_focus, cx);
            let bounds = this.map_bounds.get();
            let x = (event.position.x - bounds.origin.x).as_f32();
            let y = (event.position.y - bounds.origin.y).as_f32();
            match event.button {
                MouseButton::Middle => {
                    this.map_drag = Some((x, y));
                }
                MouseButton::Navigate(NavigationDirection::Back) => {
                    this.dispatch(Command::MapBack, cx)
                }
                MouseButton::Navigate(NavigationDirection::Forward) => {
                    this.dispatch(Command::MapForward, cx)
                }
                MouseButton::Left | MouseButton::Right => {
                    let target = pointer_hit(&this.map_tiles.borrow(), this.transform, (x, y));
                    this.pointer_tile = target;
                    this.keyboard_target = false;
                    if let Some(index) = target {
                        let focus_changed = this.focused_tile != Some(index);
                        this.focused_tile = Some(index);
                        if event.button == MouseButton::Right {
                            if focus_changed {
                                this.dispatch(Command::MapFocus(index), cx);
                            }
                            if let Some(path) = this.runtime.map_path(index) {
                                this.dispatch(Command::Open(path), cx);
                            }
                            cx.notify();
                            return;
                        }
                        this.dispatch(
                            if event.modifiers.control {
                                Command::MapMark(index)
                            } else if event.click_count >= 2 {
                                Command::MapNavigate(index)
                            } else {
                                Command::MapFocus(index)
                            },
                            cx,
                        );
                    }
                }
            }
        }))
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
            if this.sidebar_drag {
                return;
            }
            let bounds = this.map_bounds.get();
            let x = (event.position.x - bounds.origin.x).as_f32();
            let y = (event.position.y - bounds.origin.y).as_f32();
            if event.pressed_button != Some(MouseButton::Middle) {
                this.map_drag = None;
                let target = pointer_hit(&this.map_tiles.borrow(), this.transform, (x, y));
                update_pointer(this, target, cx);
                return;
            }
            if let Some((old_x, old_y)) = this.map_drag {
                this.transform.pan(x - old_x, y - old_y);
                this.map_drag = Some((x, y));
                let target = pointer_hit(&this.map_tiles.borrow(), this.transform, (x, y));
                update_pointer(this, target, cx);
                cx.notify();
            }
        }))
        .on_mouse_up(
            MouseButton::Middle,
            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                this.map_drag = None;
                cx.notify();
            }),
        )
        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
            let lines = match event.delta {
                ScrollDelta::Lines(delta) => delta.y,
                ScrollDelta::Pixels(delta) => delta.y.as_f32() / 24.,
            };
            let bounds = this.map_bounds.get();
            let x = (event.position.x - bounds.origin.x).as_f32();
            let y = (event.position.y - bounds.origin.y).as_f32();
            let tiles: Vec<_> = this
                .map_tiles
                .borrow()
                .iter()
                .map(|tile| (hit_rect(tile), tile.directory))
                .collect();
            let action = wheel_action(
                this.transform,
                lines,
                event.modifiers.shift,
                (bounds.size.width.as_f32(), bounds.size.height.as_f32()),
                (x, y),
                &tiles,
            );
            match action {
                WheelAction::Ignore => {}
                WheelAction::Pan { dy } => {
                    this.transform.pan(0., dy);
                    cx.notify();
                }
                WheelAction::Zoom { scale } => {
                    this.transform.zoom_at(x, y, scale);
                    this.dispatch(Command::MapZoom(this.transform.scale), cx);
                }
                WheelAction::Navigate(index) => this.dispatch(Command::MapNavigate(index), cx),
                WheelAction::Up => this.dispatch(Command::MapUp, cx),
            }
        }))
        .on_key_down(
            cx.listener(|this, event: &KeyDownEvent, window, cx| this.map_key(event, window, cx)),
        )
        .child(
            canvas(
                move |bounds, window, _| {
                    if measured.get() != bounds {
                        measured.set(bounds);
                        window.request_animation_frame();
                    }
                    bounds.size
                },
                move |bounds, _, window, cx| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for tile in &tiles {
                            let rect = Bounds::new(
                                Point::new(
                                    bounds.origin.x + px((tile.x - transform.x) * transform.scale),
                                    bounds.origin.y + px((tile.y - transform.y) * transform.scale),
                                ),
                                Size::new(
                                    px((tile.width * transform.scale - 1.).max(0.)),
                                    px((tile.height * transform.scale - 1.).max(0.)),
                                ),
                            );
                            if rect.size.width <= px(0.)
                                || rect.size.height <= px(0.)
                                || rect.right() < bounds.left()
                                || rect.bottom() < bounds.top()
                                || rect.left() > bounds.right()
                                || rect.top() > bounds.bottom()
                            {
                                continue;
                            }
                            let fill = tile_color(tile, color_mode);
                            let border = if tile.marked || tile.covered {
                                gpui_kit::rgb(0xff9f43).into()
                            } else if tile.matched {
                                gpui_kit::rgb(0xf0ca62).into()
                            } else if focused == Some(tile.index) {
                                palette.text
                            } else {
                                palette.border.opacity(0.4)
                            };
                            window.paint_quad(quad(
                                rect,
                                Corners::default(),
                                fill,
                                Edges::all(px(
                                    if tile.marked
                                        || tile.covered
                                        || tile.matched
                                        || focused == Some(tile.index)
                                    {
                                        2.
                                    } else {
                                        0.5
                                    },
                                )),
                                border,
                                if tile.covered && !tile.marked {
                                    BorderStyle::Dashed
                                } else {
                                    BorderStyle::Solid
                                },
                            ));
                            if color_mode == 0 && tile.reclaim {
                                let width = rect.size.width.as_f32();
                                let height = rect.size.height.as_f32();
                                let step = ((width + height) / 96.).max(14.);
                                let mut hatch = PathBuilder::stroke(px(1.));
                                let mut offset = step;
                                while offset < width + height {
                                    let start_x = (offset - height).max(0.);
                                    let start_y = (offset - width).max(0.);
                                    hatch.move_to(Point::new(
                                        rect.left() + px(start_x),
                                        rect.top() + px(offset.min(height)),
                                    ));
                                    hatch.line_to(Point::new(
                                        rect.left() + px(offset.min(width)),
                                        rect.top() + px(start_y),
                                    ));
                                    offset += step;
                                }
                                if let Ok(path) = hatch.build() {
                                    window.paint_path(path, palette.text.opacity(0.16));
                                }
                            }
                            if rect.size.width > px(55. * interface_scale)
                                && rect.size.height > px(20. * interface_scale)
                            {
                                let font = Font {
                                    family: "Segoe UI".into(),
                                    weight: if tile.directory {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    },
                                    ..Default::default()
                                };
                                let color: Hsla = gpui_kit::rgb(0xffffff).into();
                                let run = TextRun {
                                    len: tile.name.len(),
                                    font,
                                    color,
                                    ..Default::default()
                                };
                                let line = window.text_system().shape_line(
                                    SharedString::from(tile.name.clone()),
                                    px(font_size),
                                    &[run],
                                    None,
                                );
                                window.with_content_mask(
                                    Some(ContentMask { bounds: rect }),
                                    |window| {
                                        let _ = line.paint(
                                            Point::new(
                                                rect.origin.x + px(5. * interface_scale),
                                                rect.origin.y + px(3. * interface_scale),
                                            ),
                                            px(16. * interface_scale),
                                            TextAlign::Left,
                                            None,
                                            window,
                                            cx,
                                        );
                                    },
                                );
                            }
                        }
                    });
                },
            )
            .absolute()
            .inset_0(),
        )
}
pub fn category_color(category: u8) -> Hsla {
    gpui_kit::rgb(match category {
        0 => 0x397c70,
        1 => 0x805591,
        2 => 0x465fa1,
        3 => 0x5d7554,
        4 => 0x946342,
        5 => 0x91576e,
        6 => 0x9a8343,
        7 => 0x3b7991,
        _ => 0x456779,
    })
    .into()
}
pub fn age_color(age: u8) -> Hsla {
    gpui_kit::rgb(age_rgb(age)).into()
}
fn tile_color(tile: &MapTile, mode: u8) -> Hsla {
    let color = if mode == 1 {
        age_color(tile.age)
    } else {
        category_color(tile.category)
    };
    color.opacity((1. - tile.depth.min(6) as f32 * 0.045).max(0.65))
}

fn hit_rect(tile: &MapTile) -> HitRect {
    HitRect {
        index: tile.index,
        x: tile.x,
        y: tile.y,
        width: tile.width,
        height: tile.height,
        depth: tile.depth,
    }
}

fn pointer_hit(tiles: &[MapTile], transform: ViewTransform, pointer: (f32, f32)) -> Option<usize> {
    let hits: Vec<_> = tiles.iter().map(hit_rect).collect();
    let (x, y) = transform.base(pointer.0, pointer.1);
    hit_test(&hits, x, y)
}
fn update_pointer(app: &mut App, target: Option<usize>, cx: &mut Context<App>) {
    let changed = app.pointer_tile != target || app.keyboard_target;
    app.pointer_tile = target;
    app.keyboard_target = false;
    if let Some(index) = target.filter(|target| app.focused_tile != Some(*target)) {
        app.focused_tile = Some(index);
        app.dispatch(Command::MapFocus(index), cx);
    } else if changed {
        cx.notify();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn wheel_rect(index: usize, x: f32, y: f32, width: f32, height: f32, depth: u32) -> HitRect {
        HitRect {
            index,
            x,
            y,
            width,
            height,
            depth,
        }
    }

    #[test]
    fn pure_age_palette_separates_recent_old_and_unknown_metadata() {
        assert_eq!(age_rgb(0), 0x697784);
        assert_eq!(age_rgb(1), 0x397c70);
        assert_eq!(age_rgb(2), 0x487caf);
        assert_eq!(age_rgb(3), 0x8a8545);
        assert_eq!(age_rgb(4), 0xac733e);
        assert_eq!(age_rgb(5), 0x8f4b62);
        assert_eq!(age_rgb(u8::MAX), 0x697784);
        assert_eq!(label_font_size(75), 8.25);
        assert_eq!(label_font_size(150), 16.5);
    }

    #[test]
    fn wheel_enters_only_after_a_directory_already_covers_the_canvas() {
        let directory = wheel_rect(7, 250., 250., 500., 500., 0);
        let tiles = [(directory, true)];
        assert_eq!(
            wheel_action(
                ViewTransform::default(),
                1.,
                false,
                (1000., 1000.),
                (500., 500.),
                &tiles
            ),
            WheelAction::Zoom { scale: 1.15 }
        );
        let mut transform = ViewTransform::default();
        transform.zoom_at(500., 500., 2.);
        assert_eq!(
            wheel_action(transform, 1., false, (1000., 1000.), (500., 500.), &tiles),
            WheelAction::Navigate(7)
        );
        let shifted = ViewTransform {
            scale: 2.,
            x: 0.,
            y: 0.,
        };
        assert_eq!(
            wheel_action(shifted, 1., false, (1000., 1000.), (500., 500.), &tiles),
            WheelAction::Zoom { scale: 2.3 }
        );
    }

    #[test]
    fn wheel_uses_the_deepest_filled_directory_never_a_file() {
        let tiles = [
            (wheel_rect(1, 0., 0., 1000., 1000., 0), true),
            (wheel_rect(2, 0., 0., 1000., 1000., 1), true),
            (wheel_rect(3, 0., 0., 1000., 1000., 2), false),
        ];
        assert_eq!(
            wheel_action(
                ViewTransform::default(),
                1.,
                false,
                (1000., 1000.),
                (500., 500.),
                &tiles
            ),
            WheelAction::Navigate(2)
        );
        assert_eq!(
            wheel_action(
                ViewTransform::default(),
                1.,
                false,
                (1000., 1000.),
                (500., 500.),
                &tiles[2..]
            ),
            WheelAction::Zoom { scale: 1.15 }
        );
    }

    #[test]
    fn wheel_out_at_base_goes_up_and_shift_remains_pan() {
        assert_eq!(
            wheel_action(
                ViewTransform::default(),
                -1.,
                false,
                (1000., 1000.),
                (500., 500.),
                &[]
            ),
            WheelAction::Up
        );
        assert_eq!(
            wheel_action(
                ViewTransform {
                    scale: 2.,
                    x: 0.,
                    y: 0.
                },
                -1.,
                false,
                (1000., 1000.),
                (500., 500.),
                &[]
            ),
            WheelAction::Zoom { scale: 2. / 1.15 }
        );
        assert_eq!(
            wheel_action(
                ViewTransform::default(),
                -2.,
                true,
                (1000., 1000.),
                (500., 500.),
                &[]
            ),
            WheelAction::Pan { dy: -60. }
        );
    }

    #[test]
    fn wheel_rejects_nonfinite_or_outside_canvas_events() {
        for (lines, viewport, pointer) in [
            (0., (1000., 1000.), (500., 500.)),
            (f32::NAN, (1000., 1000.), (500., 500.)),
            (1., (0., 1000.), (500., 500.)),
            (1., (1000., 1000.), (1000., 500.)),
            (1., (1000., 1000.), (f32::NAN, 500.)),
        ] {
            assert_eq!(
                wheel_action(
                    ViewTransform::default(),
                    lines,
                    false,
                    viewport,
                    pointer,
                    &[]
                ),
                WheelAction::Ignore
            );
        }
    }

    #[test]
    fn age_mode_uses_distinct_bands_and_category_mode_preserves_categories() {
        let tile = MapTile {
            index: 1,
            name: "data".into(),
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
            directory: false,
            marked: false,
            covered: false,
            matched: false,
            depth: 0,
            category: 4,
            reclaim: false,
            age: 5,
        };
        assert_eq!(tile_color(&tile, 0), category_color(4));
        let expected: Hsla = gpui_kit::rgb(0x8f4b62).into();
        assert_eq!(tile_color(&tile, 1), expected);
        for left in 0..6 {
            for right in left + 1..6 {
                assert_ne!(age_color(left), age_color(right));
            }
        }
        assert_eq!(age_color(u8::MAX), age_color(0));
    }

    #[test]
    fn map_glyphs_follow_interface_scale() {
        assert_eq!(label_font_size(75), 8.25);
        assert_eq!(label_font_size(100), 11.);
        assert_eq!(label_font_size(150), 16.5);
    }
    #[test]
    fn arrows_choose_the_closest_tile_in_the_requested_direction() {
        let rect = |index, x, y| HitRect {
            index,
            x,
            y,
            width: 10.,
            height: 10.,
            depth: 0,
        };
        let tiles = [rect(1, 0., 0.), rect(2, 12., 0.), rect(3, 12., 50.)];
        assert_eq!(neighbor(&tiles, 1, (1., 0.)), Some(2));
        assert_eq!(neighbor(&tiles, 2, (-1., 0.)), Some(1));
        assert_eq!(neighbor(&tiles, 1, (-1., 0.)), None);
    }
    #[test]
    fn deepest_visible_tile_wins_and_boundary_never_hits_neighbor() {
        let tiles = [
            HitRect {
                index: 1,
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
                depth: 0,
            },
            HitRect {
                index: 2,
                x: 0.,
                y: 0.,
                width: 50.,
                height: 50.,
                depth: 1,
            },
        ];
        assert_eq!(hit_test(&tiles, 10., 10.), Some(2));
        assert_eq!(hit_test(&tiles, 50., 10.), Some(1));
        assert_eq!(hit_test(&tiles, 100., 10.), None);
        assert_eq!(hit_test(&tiles, f32::NAN, 10.), None);
    }
    #[test]
    fn zoom_preserves_pointer_anchor_and_rejects_nonfinite() {
        let mut view = ViewTransform::default();
        let before = view.base(80., 60.);
        view.zoom_at(80., 60., 2.);
        assert_eq!(view.scale, 2.);
        assert_eq!(view.base(80., 60.), before);
        view.zoom_at(80., 60., f32::INFINITY);
        assert_eq!(view.scale, 2.);
    }
}
