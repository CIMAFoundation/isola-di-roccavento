//! Pictograms drawn with the egui painter, so the interface can say with a
//! shape what it used to say with a sentence. No font glyphs: the bundled
//! font has no emoji or arrows, and a missing glyph draws as a box.

use bevy_egui::egui::{self, pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    House,
    Family,
    Engine,
    Exit,
    Cross,
    Up,
    Pause,
    Play,
    Alert,
    Check,
    Clock,
    Drop,
}

fn poly(p: &Painter, r: Rect, pts: &[(f32, f32)], fill: Color32) {
    let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| pos2(r.left() + x * r.width(), r.top() + y * r.height())).collect();
    p.add(Shape::convex_polygon(pts, fill, Stroke::NONE));
}

fn at(r: Rect, x: f32, y: f32) -> Pos2 {
    pos2(r.left() + x * r.width(), r.top() + y * r.height())
}

/// Draw `icon` filling the square `r`, in `col`.
pub fn draw(p: &Painter, r: Rect, icon: Icon, col: Color32) {
    let s = r.width().min(r.height());
    let r = Rect::from_center_size(r.center(), vec2(s, s));
    let w = (s * 0.11).max(2.0);
    let st = Stroke::new(w, col);
    match icon {
        Icon::House => {
            poly(p, r, &[(0.5, 0.08), (0.95, 0.48), (0.05, 0.48)], col);
            p.rect_filled(Rect::from_min_max(at(r, 0.17, 0.46), at(r, 0.83, 0.92)), 1.0, col);
        }
        Icon::Family => {
            for (x, h) in [(0.32, 1.0), (0.7, 0.75)] {
                let head = at(r, x, 0.62 - 0.4 * h);
                p.circle_filled(head, s * 0.11 * h.max(0.8), col);
                p.rect_filled(Rect::from_min_max(at(r, x - 0.14 * h, 0.72 - 0.38 * h + 0.12), at(r, x + 0.14 * h, 0.95)), s * 0.08, col);
            }
        }
        Icon::Engine => {
            p.rect_filled(Rect::from_min_max(at(r, 0.04, 0.3), at(r, 0.66, 0.74)), 2.0, col);
            p.rect_filled(Rect::from_min_max(at(r, 0.68, 0.44), at(r, 0.96, 0.74)), 2.0, col);
            for x in [0.22, 0.78] {
                p.circle_filled(at(r, x, 0.8), s * 0.11, col);
                p.circle_filled(at(r, x, 0.8), s * 0.05, Color32::from_gray(20));
            }
        }
        Icon::Exit => {
            p.rect_stroke(Rect::from_min_max(at(r, 0.08, 0.1), at(r, 0.5, 0.9)), 1.0, st, egui::StrokeKind::Middle);
            p.line_segment([at(r, 0.32, 0.5), at(r, 0.92, 0.5)], st);
            p.line_segment([at(r, 0.92, 0.5), at(r, 0.74, 0.32)], st);
            p.line_segment([at(r, 0.92, 0.5), at(r, 0.74, 0.68)], st);
        }
        Icon::Cross => {
            p.line_segment([at(r, 0.2, 0.2), at(r, 0.8, 0.8)], st);
            p.line_segment([at(r, 0.8, 0.2), at(r, 0.2, 0.8)], st);
        }
        Icon::Up => {
            p.line_segment([at(r, 0.2, 0.62), at(r, 0.5, 0.28)], st);
            p.line_segment([at(r, 0.5, 0.28), at(r, 0.8, 0.62)], st);
            p.line_segment([at(r, 0.2, 0.86), at(r, 0.8, 0.86)], st);
        }
        Icon::Pause => {
            p.rect_filled(Rect::from_min_max(at(r, 0.22, 0.18), at(r, 0.42, 0.82)), 1.0, col);
            p.rect_filled(Rect::from_min_max(at(r, 0.58, 0.18), at(r, 0.78, 0.82)), 1.0, col);
        }
        Icon::Play => {
            poly(p, r, &[(0.25, 0.15), (0.85, 0.5), (0.25, 0.85)], col);
        }
        Icon::Alert => {
            poly(p, r, &[(0.5, 0.05), (0.97, 0.92), (0.03, 0.92)], col);
            p.line_segment([at(r, 0.5, 0.35), at(r, 0.5, 0.65)], Stroke::new(w, Color32::BLACK));
            p.circle_filled(at(r, 0.5, 0.78), w * 0.6, Color32::BLACK);
        }
        Icon::Clock => {
            p.circle_stroke(r.center(), s * 0.42, st);
            p.line_segment([r.center(), at(r, 0.5, 0.22)], st);
            p.line_segment([r.center(), at(r, 0.7, 0.6)], st);
        }
        Icon::Drop => {
            poly(p, r, &[(0.5, 0.05), (0.72, 0.45), (0.28, 0.45)], col);
            p.circle_filled(at(r, 0.5, 0.62), s * 0.27, col);
        }
        Icon::Check => {
            p.line_segment([at(r, 0.15, 0.55), at(r, 0.4, 0.8)], st);
            p.line_segment([at(r, 0.4, 0.8), at(r, 0.88, 0.25)], st);
        }
    }
}

/// An icon on its own, `size` px square.
pub fn show(ui: &mut egui::Ui, icon: Icon, size: f32, col: Color32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    draw(ui.painter(), r, icon, col);
    resp
}

/// A hand over what can be pressed (playtest colleghi: the arrow over the
/// level buttons did not say they were buttons). egui's own buttons get it
/// from `Visuals::interact_cursor`; drawn ones call this.
pub fn pointing(resp: egui::Response) -> egui::Response {
    if resp.enabled() && resp.sense.senses_click() {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}
