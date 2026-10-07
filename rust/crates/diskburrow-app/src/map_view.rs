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

use crate::{
    contract::{Command, MapTile},
    ui::{App, Palette},
};
use gpui_kit::{
    BorderStyle, Bounds, ContentMask, Context, Corners, Edges, Font, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, NavigationDirection, ParentElement as _, Point, ScrollDelta,
    ScrollWheelEvent, SharedString, Size, StatefulInteractiveElement as _, Styled,
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
    let focused = app.focused_tile;
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
                MouseButton::Left => {
                    let (x, y) = this.transform.base(x, y);
                    let hits = this
                        .map_tiles
                        .borrow()
                        .iter()
                        .map(|tile| HitRect {
                            index: tile.index,
                            x: tile.x,
                            y: tile.y,
                            width: tile.width,
                            height: tile.height,
                            depth: tile.depth,
                        })
                        .collect::<Vec<_>>();
                    if let Some(index) = hit_test(&hits, x, y) {
                        this.focused_tile = Some(index);
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
                _ => {}
            }
        }))
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
            if event.pressed_button != Some(MouseButton::Middle) {
                this.map_drag = None;
                return;
            }
            if let Some((old_x, old_y)) = this.map_drag {
                let bounds = this.map_bounds.get();
                let x = (event.position.x - bounds.origin.x).as_f32();
                let y = (event.position.y - bounds.origin.y).as_f32();
                this.transform.pan(x - old_x, y - old_y);
                this.map_drag = Some((x, y));
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
            if lines.abs() < f32::EPSILON {
                return;
            }
            if event.modifiers.shift {
                this.transform.pan(0., lines * 30.);
                cx.notify();
                return;
            }
            let bounds = this.map_bounds.get();
            let x = (event.position.x - bounds.origin.x).as_f32();
            let y = (event.position.y - bounds.origin.y).as_f32();
            this.transform.zoom_at(
                x,
                y,
                this.transform.scale * if lines > 0. { 1.15 } else { 1. / 1.15 },
            );
            this.dispatch(Command::MapZoom(this.transform.scale), cx);
        }))
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.map_key(event, cx)))
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
                            let fill = tile_color(tile);
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
                            if rect.size.width > px(55.) && rect.size.height > px(20.) {
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
                                    px(11.),
                                    &[run],
                                    None,
                                );
                                window.with_content_mask(
                                    Some(ContentMask { bounds: rect }),
                                    |window| {
                                        let _ = line.paint(
                                            Point::new(
                                                rect.origin.x + px(5.),
                                                rect.origin.y + px(3.),
                                            ),
                                            px(16.),
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
fn tile_color(tile: &MapTile) -> Hsla {
    let extension = std::path::Path::new(&tile.path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let color = if tile.directory {
        0x365775
    } else {
        match extension.as_str() {
            "mp4" | "mkv" | "mp3" | "wav" | "flac" => 0x805591,
            "png" | "jpg" | "jpeg" | "webp" => 0x397c70,
            "zip" | "7z" | "rar" | "gz" => 0x946342,
            "exe" | "dll" | "msi" => 0x465fa1,
            "pdf" | "docx" | "txt" | "md" => 0x5d7554,
            _ => 0x456779,
        }
    };
    gpui_kit::Hsla::from(gpui_kit::rgb(color))
        .opacity((1. - tile.depth.min(6) as f32 * 0.045).max(0.65))
}
#[cfg(test)]
mod tests {
    use super::*;
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
