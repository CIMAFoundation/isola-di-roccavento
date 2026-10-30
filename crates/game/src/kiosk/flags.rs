//! Small flags for the language panel (user, 2026-10-30), painted with egui
//! shapes: the UI fonts have no emoji flags. Each is simplified to stay
//! readable at about 36×24 px: stripes, crosses and a few plain emblems.
//! A language spoken in more than one country takes one flag (see
//! `docs/01-GIOCO.md` §7); English is the United Kingdom's, Catalan, Galician
//! and Basque have their own. A code without a flag gets a grey card with «?».

use bevy_egui::egui::{self, pos2, Color32, Pos2, Rect, Shape, Stroke};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const WHITE: Color32 = Color32::WHITE;
const BLACK: Color32 = Color32::BLACK;

/// Paints the flag of language `code` inside `rect` (3:2 looks right), with a
/// thin dark border so white stripes stay visible on a light background.
/// Returns whether `code` has a flag of its own.
pub fn paint(painter: &egui::Painter, rect: Rect, code: &str) -> bool {
    let mut own = true;
    let p = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let f = Flag { p: &p, r: rect };
    match code {
        "it" => f.v(&[rgb(0, 146, 70), WHITE, rgb(206, 43, 55)]),
        "en" => f.union_jack(),
        "de" => f.h(&[BLACK, rgb(221, 0, 0), rgb(255, 206, 0)]),
        "fr" => f.v(&[rgb(0, 85, 164), WHITE, rgb(239, 65, 53)]),
        "es" => f.hb(&[(1.0, rgb(170, 21, 27)), (2.0, rgb(241, 191, 0)), (1.0, rgb(170, 21, 27))]),
        "pt" => {
            f.vb(&[(2.0, rgb(0, 102, 0)), (3.0, rgb(255, 0, 0))]);
            p.circle_filled(f.at(0.4, 0.5), f.h_(0.22), rgb(255, 221, 0));
            p.circle_filled(f.at(0.4, 0.5), f.h_(0.11), WHITE);
        }
        "ca" => {
            let (y, r) = (rgb(252, 221, 9), rgb(218, 18, 26));
            f.h(&[y, r, y, r, y, r, y, r, y]);
        }
        "gl" => {
            f.fill(WHITE);
            p.line_segment([f.at(0.0, 0.0), f.at(1.0, 1.0)], Stroke::new(f.h_(0.26), rgb(0, 153, 204)));
        }
        "eu" => {
            f.fill(rgb(213, 43, 30));
            f.saltire(f.h_(0.16), rgb(0, 150, 63));
            f.cross(0.5, f.h_(0.12), WHITE);
        }
        "nl" => f.h(&[rgb(174, 28, 40), WHITE, rgb(33, 70, 139)]),
        "lb" => f.h(&[rgb(237, 41, 57), WHITE, rgb(0, 161, 222)]),
        "da" => f.nordic(rgb(200, 16, 46), WHITE, None),
        "sv" => f.nordic(rgb(0, 106, 167), rgb(254, 204, 0), None),
        "nb" => f.nordic(rgb(186, 12, 47), WHITE, Some(rgb(0, 32, 91))),
        "is" => f.nordic(rgb(2, 82, 156), WHITE, Some(rgb(220, 30, 53))),
        "fi" => f.nordic(WHITE, rgb(0, 47, 108), None),
        "et" => f.h(&[rgb(0, 114, 206), BLACK, WHITE]),
        "lv" => f.hb(&[(2.0, rgb(158, 48, 57)), (1.0, WHITE), (2.0, rgb(158, 48, 57))]),
        "lt" => f.h(&[rgb(253, 185, 19), rgb(0, 106, 68), rgb(193, 39, 45)]),
        "pl" => f.h(&[WHITE, rgb(220, 20, 60)]),
        "cs" => {
            f.h(&[WHITE, rgb(215, 20, 26)]);
            f.poly(&[(0.0, 0.0), (0.5, 0.5), (0.0, 1.0)], rgb(17, 69, 126));
        }
        "sk" => {
            f.h(&[WHITE, rgb(11, 78, 162), rgb(238, 28, 37)]);
            f.shield(0.33, 0.5, 0.36, rgb(238, 28, 37), WHITE);
            // the double cross, as one bar and two arms
            p.line_segment([f.at(0.33, 0.3), f.at(0.33, 0.62)], Stroke::new(f.h_(0.05), WHITE));
            p.line_segment([f.at(0.29, 0.4), f.at(0.37, 0.4)], Stroke::new(f.h_(0.05), WHITE));
            p.line_segment([f.at(0.27, 0.5), f.at(0.39, 0.5)], Stroke::new(f.h_(0.05), WHITE));
        }
        "sl" => {
            f.h(&[WHITE, rgb(0, 94, 184), rgb(237, 28, 36)]);
            f.shield(0.27, 0.36, 0.34, rgb(0, 94, 184), rgb(237, 28, 36));
            f.poly(&[(0.21, 0.44), (0.27, 0.3), (0.33, 0.44)], WHITE);
        }
        "sr" => {
            f.h(&[rgb(198, 54, 60), rgb(12, 64, 118), WHITE]);
            f.shield(0.3, 0.5, 0.42, rgb(198, 54, 60), rgb(240, 200, 60));
            f.cross_at(f.at(0.3, 0.47), f.h_(0.14), f.h_(0.05), WHITE);
        }
        "bs" => {
            f.fill(rgb(0, 35, 149));
            f.poly(&[(0.28, 0.0), (0.76, 0.0), (0.76, 1.0)], rgb(254, 203, 0));
            for i in 0..6 {
                let t = i as f32 / 5.0;
                p.circle_filled(f.at(0.17 + 0.48 * t, 0.06 + 0.88 * t), f.h_(0.055), WHITE);
            }
        }
        "mk" => {
            f.fill(rgb(206, 32, 39));
            let c = f.at(0.5, 0.5);
            let sun = rgb(248, 233, 46);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = egui::vec2(a.cos(), a.sin());
                let n = egui::vec2(-d.y, d.x) * f.h_(0.09);
                let far = c + d * f.r.width();
                p.add(Shape::convex_polygon(vec![c - n, c + n, far], sun, Stroke::NONE));
            }
            p.circle(c, f.h_(0.17), sun, Stroke::new(f.h_(0.04), rgb(206, 32, 39)));
        }
        "sq" => {
            f.fill(rgb(228, 30, 32));
            // the double-headed eagle as a black silhouette: body, wings, heads
            f.poly(&[(0.5, 0.22), (0.56, 0.5), (0.5, 0.86), (0.44, 0.5)], BLACK);
            f.poly(&[(0.5, 0.42), (0.74, 0.22), (0.7, 0.56), (0.5, 0.62)], BLACK);
            f.poly(&[(0.5, 0.42), (0.26, 0.22), (0.3, 0.56), (0.5, 0.62)], BLACK);
            p.circle_filled(f.at(0.44, 0.18), f.h_(0.06), BLACK);
            p.circle_filled(f.at(0.56, 0.18), f.h_(0.06), BLACK);
        }
        "mt" => {
            f.v(&[WHITE, rgb(207, 20, 43)]);
            let c = f.at(0.12, 0.17);
            p.rect(Rect::from_center_size(c, egui::vec2(f.h_(0.18), f.h_(0.18))), 1.0, rgb(170, 170, 170), Stroke::new(1.0, rgb(207, 20, 43)), egui::StrokeKind::Inside);
        }
        "ga" => f.v(&[rgb(22, 155, 98), WHITE, rgb(255, 136, 62)]),
        "cy" => {
            f.h(&[WHITE, rgb(0, 177, 64)]);
            // the red dragon, as a crouching shape with a raised head and tail
            let red = rgb(200, 16, 46);
            f.poly(&[(0.3, 0.42), (0.68, 0.42), (0.74, 0.62), (0.3, 0.7)], red);
            f.poly(&[(0.6, 0.44), (0.72, 0.18), (0.8, 0.26), (0.7, 0.48)], red);
            f.poly(&[(0.3, 0.5), (0.18, 0.28), (0.24, 0.26), (0.36, 0.44)], red);
            f.poly(&[(0.36, 0.66), (0.42, 0.84), (0.48, 0.66)], red);
            f.poly(&[(0.6, 0.64), (0.66, 0.84), (0.72, 0.62)], red);
        }
        "uk" => f.h(&[rgb(0, 87, 183), rgb(255, 215, 0)]),
        "be" => {
            f.hb(&[(2.0, rgb(200, 49, 62)), (1.0, rgb(76, 160, 70))]);
            p.rect_filled(Rect::from_min_max(f.at(0.0, 0.0), f.at(0.12, 1.0)), 0.0, WHITE);
            for i in 0..4 {
                let c = f.at(0.06, 0.125 + 0.25 * i as f32);
                let (dx, dy) = (f.r.width() * 0.04, f.h_(0.1));
                p.add(Shape::convex_polygon(vec![c + egui::vec2(0.0, -dy), c + egui::vec2(dx, 0.0), c + egui::vec2(0.0, dy), c + egui::vec2(-dx, 0.0)], rgb(200, 49, 62), Stroke::NONE));
            }
        }
        "ru" => f.h(&[WHITE, rgb(0, 57, 166), rgb(213, 43, 30)]),
        "tr" => {
            let red = rgb(227, 10, 23);
            f.fill(red);
            p.circle_filled(f.at(0.38, 0.5), f.h_(0.26), WHITE);
            p.circle_filled(f.at(0.43, 0.5), f.h_(0.21), red);
            f.star(f.at(0.63, 0.5), f.h_(0.12), WHITE);
        }
        "hu" => f.h(&[rgb(206, 41, 57), WHITE, rgb(71, 112, 80)]),
        "ro" => f.v(&[rgb(0, 43, 127), rgb(252, 209, 22), rgb(206, 17, 38)]),
        "bg" => f.h(&[WHITE, rgb(0, 150, 110), rgb(214, 38, 18)]),
        "el" => {
            let blue = rgb(13, 94, 175);
            f.h(&[blue, WHITE, blue, WHITE, blue, WHITE, blue, WHITE, blue]);
            let side = f.r.height() * 5.0 / 9.0;
            let canton = Rect::from_min_size(f.r.min, egui::vec2(side, side));
            p.rect_filled(canton, 0.0, blue);
            let bar = f.r.height() / 9.0;
            p.rect_filled(Rect::from_center_size(canton.center(), egui::vec2(side, bar)), 0.0, WHITE);
            p.rect_filled(Rect::from_center_size(canton.center(), egui::vec2(bar, side)), 0.0, WHITE);
        }
        "hr" => {
            f.h(&[rgb(255, 0, 0), WHITE, rgb(23, 23, 150)]);
            // the chequered shield, 5 by 5
            let s = f.h_(0.09);
            let o = f.at(0.5, 0.5) - egui::vec2(2.5 * s, 2.5 * s);
            p.rect_filled(Rect::from_min_size(o, egui::vec2(5.0 * s, 5.0 * s)).expand(1.0), 1.0, WHITE);
            for i in 0..5 {
                for j in 0..5 {
                    let c = if (i + j) % 2 == 0 { rgb(255, 0, 0) } else { WHITE };
                    p.rect_filled(Rect::from_min_size(o + egui::vec2(i as f32 * s, j as f32 * s), egui::vec2(s, s)), 0.0, c);
                }
            }
        }
        _ => {
            own = false;
            f.fill(rgb(90, 90, 96));
            p.text(rect.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(rect.height() * 0.7), WHITE);
        }
    }
    painter.rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::from_black_alpha(160)), egui::StrokeKind::Inside);
    own
}

struct Flag<'a> {
    p: &'a egui::Painter,
    r: Rect,
}

impl Flag<'_> {
    /// A point in flag units: `(0, 0)` top left, `(1, 1)` bottom right.
    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.left() + x * self.r.width(), self.r.top() + y * self.r.height())
    }

    /// A length as a fraction of the flag's height.
    fn h_(&self, f: f32) -> f32 {
        f * self.r.height()
    }

    fn fill(&self, c: Color32) {
        self.p.rect_filled(self.r, 0.0, c);
    }

    /// Equal horizontal stripes, top to bottom.
    fn h(&self, cs: &[Color32]) {
        self.hb(&cs.iter().map(|c| (1.0, *c)).collect::<Vec<_>>());
    }

    /// Horizontal bands of the given weights.
    fn hb(&self, bands: &[(f32, Color32)]) {
        let total: f32 = bands.iter().map(|b| b.0).sum();
        let mut y = 0.0;
        for (w, c) in bands {
            self.p.rect_filled(Rect::from_min_max(self.at(0.0, y / total), self.at(1.0, (y + w) / total)), 0.0, *c);
            y += w;
        }
    }

    /// Equal vertical stripes, left to right.
    fn v(&self, cs: &[Color32]) {
        self.vb(&cs.iter().map(|c| (1.0, *c)).collect::<Vec<_>>());
    }

    fn vb(&self, bands: &[(f32, Color32)]) {
        let total: f32 = bands.iter().map(|b| b.0).sum();
        let mut x = 0.0;
        for (w, c) in bands {
            self.p.rect_filled(Rect::from_min_max(self.at(x / total, 0.0), self.at((x + w) / total, 1.0)), 0.0, *c);
            x += w;
        }
    }

    /// A filled polygon in flag units (convex).
    fn poly(&self, pts: &[(f32, f32)], c: Color32) {
        self.p.add(Shape::convex_polygon(pts.iter().map(|(x, y)| self.at(*x, *y)).collect(), c, Stroke::NONE));
    }

    /// An upright cross through `(x, 0.5)` of bar width `w`.
    fn cross(&self, x: f32, w: f32, c: Color32) {
        let cx = self.at(x, 0.5).x;
        self.p.rect_filled(Rect::from_min_max(pos2(cx - w / 2.0, self.r.top()), pos2(cx + w / 2.0, self.r.bottom())), 0.0, c);
        let cy = self.r.center().y;
        self.p.rect_filled(Rect::from_min_max(pos2(self.r.left(), cy - w / 2.0), pos2(self.r.right(), cy + w / 2.0)), 0.0, c);
    }

    /// A small cross centred on `c`.
    fn cross_at(&self, c: Pos2, size: f32, w: f32, col: Color32) {
        self.p.rect_filled(Rect::from_center_size(c, egui::vec2(w, size)), 0.0, col);
        self.p.rect_filled(Rect::from_center_size(c, egui::vec2(size, w)), 0.0, col);
    }

    /// The diagonal cross, corner to corner.
    fn saltire(&self, w: f32, c: Color32) {
        self.p.line_segment([self.at(0.0, 0.0), self.at(1.0, 1.0)], Stroke::new(w, c));
        self.p.line_segment([self.at(0.0, 1.0), self.at(1.0, 0.0)], Stroke::new(w, c));
    }

    /// The Nordic cross, shifted to the hoist, with an optional inner cross.
    fn nordic(&self, bg: Color32, cross: Color32, inner: Option<Color32>) {
        self.fill(bg);
        self.cross(0.37, self.h_(0.26), cross);
        if let Some(i) = inner {
            self.cross(0.37, self.h_(0.12), i);
        }
    }

    /// A small heater shield centred at `(x, y)` (flag units), `h` tall as a
    /// fraction of the flag's height.
    fn shield(&self, x: f32, y: f32, h: f32, fill: Color32, edge: Color32) {
        let c = self.at(x, y);
        let (hh, hw) = (self.h_(h) / 2.0, self.h_(h) * 0.4);
        let pts = vec![c + egui::vec2(-hw, -hh), c + egui::vec2(hw, -hh), c + egui::vec2(hw, hh * 0.3), c + egui::vec2(0.0, hh), c + egui::vec2(-hw, hh * 0.3)];
        self.p.add(Shape::convex_polygon(pts, fill, Stroke::new(1.0, edge)));
    }

    /// A five-pointed star: a pentagon and its five points.
    fn star(&self, c: Pos2, r: f32, col: Color32) {
        let pt = |i: usize, rad: f32| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / 10.0;
            c + egui::vec2(a.cos(), a.sin()) * rad
        };
        let inner = r * 0.4;
        self.p.add(Shape::convex_polygon((0..5).map(|i| pt(2 * i + 1, inner)).collect(), col, Stroke::NONE));
        for i in 0..5 {
            self.p.add(Shape::convex_polygon(vec![pt(2 * i, r), pt(2 * i + 1, inner), pt((2 * i + 9) % 10, inner)], col, Stroke::NONE));
        }
    }

    /// The Union Jack, simplified: no counterchange of the red diagonals.
    fn union_jack(&self) {
        let (blue, red) = (rgb(1, 33, 105), rgb(200, 16, 46));
        self.fill(blue);
        self.saltire(self.h_(0.2), WHITE);
        self.saltire(self.h_(0.07), red);
        self.cross(0.5, self.h_(0.33), WHITE);
        self.cross(0.5, self.h_(0.2), red);
    }
}

#[cfg(test)]
mod tests {
    use bevy_egui::egui;

    #[test]
    fn every_language_but_the_easter_egg_has_a_flag() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            let p = ui.ctx().layer_painter(egui::LayerId::background());
            let r = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(36.0, 24.0));
            for (code, _) in rocca::i18n::LANGS {
                assert!(super::paint(&p, r, code), "{code}");
            }
            assert!(!super::paint(&p, r, rocca::i18n::EASTER_EGG.0));
        });
    }
}
