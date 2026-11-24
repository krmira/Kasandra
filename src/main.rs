#![windows_subsystem = "windows"] // no console window on Windows

// Kasandra - a tiny multi-provider chat client. One file, egui.
// Providers: OpenAI-compatible (OpenAI, Groq, OpenRouter, xAI, Cerebras, custom), Anthropic, Gemini.
//
// Cargo.toml additions:
//   serde   = { version = "1", features = ["derive"] }
//   dirs    = "5"
//   keyring = { version = "3", features = ["windows-native", "apple-native", "sync-secret-service"] }

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Area, Color32, Context, CursorIcon, FontId, Id, Key, Layout,
    Margin, Order, Painter, Pos2, Rect, Response, RichText, Rounding, Sense, Shape, Stroke,
    TextureHandle, Ui,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

const TITLE_H: f32 = 34.0;
const SIDEBAR_W: f32 = 290.0;
const WIN_R: f32 = 18.0; // window corner radius
const PANEL_R: f32 = 20.0; // glass panel radius
const GLASS_A: f32 = 77.0; // 30% white tint over the frosted wallpaper
const GRAIN_A: u8 = 0; // acrylic noise strength (0 = off)
const ICON_PNG: &[u8] = include_bytes!("../kasandra.png");
const ACCENT: Color32 = Color32::from_rgb(10, 140, 255);
const BOT_H: f32 = 118.0; // height of the background controls at the bottom of the sidebar
const BLUR_MAX: f32 = 1.0;
const BLUR_PX: f32 = 32.0;
const BG_W: usize = 960;
const BG_H: usize = 540;
const GREEN: Color32 = Color32::from_rgb(46, 204, 98);
const GREEN_DK: Color32 = Color32::from_rgb(30, 168, 78);
const INK: Color32 = Color32::from_rgb(20, 26, 42);
const RED: Color32 = Color32::from_rgb(229, 72, 77);
const RED_DK: Color32 = Color32::from_rgb(196, 52, 58);

const PROJ_ROW: f32 = 44.0; // sidebar row heights (a bit more air than before)
const CHAT_ROW: f32 = 40.0;
const BODY_SIZE: f32 = 15.0;
const LINE_H: f32 = 23.0; // chat text line height (was egui's tight default)

const KR_SERVICE: &str = "Kasandra";

// ───────────────────────────── small helpers ─────────────────────────────

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn fa(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}
fn muted() -> Color32 {
    Color32::from_rgba_unmultiplied(20, 26, 42, 140)
}
fn lerpc(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}
fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}
fn bold_family() -> egui::FontFamily {
    egui::FontFamily::Name("bold".into())
}
fn blur_r(amt: f32) -> usize {
    (amt * amt * BLUR_PX).round() as usize
}

/// Clickable rect with an animated hover value (0..1).
fn hit(ui: &mut Ui, rect: Rect, id: Id) -> (Response, f32) {
    let r = ui.interact(rect, id, Sense::click());
    let h = ui.ctx().animate_bool_with_time(id.with("hover"), r.hovered(), 0.07);
    if r.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    (r, h)
}

fn draw_x(p: &Painter, c: Pos2, r: f32, col: Color32) {
    let s = Stroke::new(1.8, col);
    p.line_segment([c + vec2(-r, -r), c + vec2(r, r)], s);
    p.line_segment([c + vec2(-r, r), c + vec2(r, -r)], s);
}
fn draw_plus(p: &Painter, c: Pos2, r: f32, col: Color32) {
    let s = Stroke::new(1.8, col);
    p.line_segment([c + vec2(-r, 0.0), c + vec2(r, 0.0)], s);
    p.line_segment([c + vec2(0.0, -r), c + vec2(0.0, r)], s);
}
fn draw_check(p: &Painter, c: Pos2, r: f32, col: Color32) {
    let s = Stroke::new(2.4, col);
    p.line_segment([c + vec2(-r, 0.0), c + vec2(-r * 0.3, r * 0.75)], s);
    p.line_segment([c + vec2(-r * 0.3, r * 0.75), c + vec2(r, -r * 0.65)], s);
}
fn draw_arrow_up(p: &Painter, c: Pos2, r: f32, col: Color32) {
    let s = Stroke::new(2.2, col);
    p.line_segment([c + vec2(0.0, r), c + vec2(0.0, -r)], s);
    p.line_segment([c + vec2(-r * 0.75, -r * 0.2), c + vec2(0.0, -r)], s);
    p.line_segment([c + vec2(r * 0.75, -r * 0.2), c + vec2(0.0, -r)], s);
}

/// Everything needed to draw textured shapes that line up with the window background.
#[derive(Clone, Copy)]
struct Bg {
    sharp: egui::TextureId,
    blur: egui::TextureId,
    grain: egui::TextureId,
    screen: Rect,
    uv: Rect,
    win_r: f32,
}

fn rounded_poly(r: Rect, rad: f32, seg: usize) -> Vec<Pos2> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    if rad < 0.5 {
        return vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
    }
    let corners = [
        (pos2(r.right() - rad, r.top() + rad), -FRAC_PI_2),
        (pos2(r.right() - rad, r.bottom() - rad), 0.0),
        (pos2(r.left() + rad, r.bottom() - rad), FRAC_PI_2),
        (pos2(r.left() + rad, r.top() + rad), PI),
    ];
    let mut v = Vec::with_capacity(4 * (seg + 1));
    for (c, a0) in corners {
        for i in 0..=seg {
            let a = a0 + FRAC_PI_2 * (i as f32 / seg as f32);
            v.push(c + vec2(a.cos(), a.sin()) * rad);
        }
    }
    v
}

/// Fan-triangulated convex polygon with a per-vertex uv and colour.
fn fan(p: &Painter, tex: egui::TextureId, pts: &[Pos2], uv: impl Fn(Pos2) -> Pos2, col: impl Fn(Pos2) -> Color32) {
    let n = pts.len();
    if n < 3 {
        return;
    }
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    for q in pts {
        cx += q.x;
        cy += q.y;
    }
    let cen = pos2(cx / n as f32, cy / n as f32);
    let mut m = egui::Mesh::with_texture(tex);
    m.vertices.push(egui::epaint::Vertex { pos: cen, uv: uv(cen), color: col(cen) });
    for q in pts {
        m.vertices.push(egui::epaint::Vertex { pos: *q, uv: uv(*q), color: col(*q) });
    }
    for i in 0..n {
        m.indices.extend_from_slice(&[0, (1 + i) as u32, (1 + (i + 1) % n) as u32]);
    }
    p.add(Shape::mesh(m));
}

/// Convex polygon filled with the background texture at the matching screen position.
fn tex_poly(p: &Painter, bg: &Bg, tex: egui::TextureId, pts: &[Pos2], tint: Color32) {
    let uv = |q: Pos2| {
        pos2(
            bg.uv.min.x + (q.x - bg.screen.min.x) / bg.screen.width() * bg.uv.width(),
            bg.uv.min.y + (q.y - bg.screen.min.y) / bg.screen.height() * bg.uv.height(),
        )
    };
    fan(p, tex, pts, uv, |_| tint);
}

/// Sutherland-Hodgman clip of a convex polygon against an axis-aligned rect.
fn clip_poly(pts: &[Pos2], r: Rect) -> Vec<Pos2> {
    let mut out = pts.to_vec();
    for edge in 0..4 {
        let inside = |p: Pos2| match edge {
            0 => p.x >= r.left(),
            1 => p.x <= r.right(),
            2 => p.y >= r.top(),
            _ => p.y <= r.bottom(),
        };
        let isect = |a: Pos2, b: Pos2| match edge {
            0 | 1 => {
                let x = if edge == 0 { r.left() } else { r.right() };
                let t = (x - a.x) / (b.x - a.x);
                pos2(x, a.y + t * (b.y - a.y))
            }
            _ => {
                let y = if edge == 2 { r.top() } else { r.bottom() };
                let t = (y - a.y) / (b.y - a.y);
                pos2(a.x + t * (b.x - a.x), y)
            }
        };
        let inp = std::mem::take(&mut out);
        if inp.is_empty() {
            break;
        }
        for i in 0..inp.len() {
            let (a, b) = (inp[i], inp[(i + 1) % inp.len()]);
            match (inside(a), inside(b)) {
                (true, true) => out.push(b),
                (true, false) => out.push(isect(a, b)),
                (false, true) => {
                    out.push(isect(a, b));
                    out.push(b);
                }
                _ => {}
            }
        }
    }
    out
}

/// Acrylic noise: a 256px noise tile repeated over the polygon.
fn grain(p: &Painter, bg: &Bg, pts: &[Pos2], origin: Pos2, a: u8) {
    if a == 0 {
        return;
    }
    const T: f32 = 256.0;
    let (mut lo, mut hi) = (pts[0], pts[0]);
    for q in pts {
        lo = pos2(lo.x.min(q.x), lo.y.min(q.y));
        hi = pos2(hi.x.max(q.x), hi.y.max(q.y));
    }
    let tint = Color32::from_white_alpha(a);
    let (x0, x1) = (((lo.x - origin.x) / T).floor() as i32, ((hi.x - origin.x) / T).floor() as i32);
    let (y0, y1) = (((lo.y - origin.y) / T).floor() as i32, ((hi.y - origin.y) / T).floor() as i32);
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let tile = Rect::from_min_size(origin + vec2(tx as f32 * T, ty as f32 * T), vec2(T, T));
            let c = clip_poly(pts, tile);
            fan(p, bg.grain, &c, |q| pos2((q.x - tile.left()) / T, (q.y - tile.top()) / T), |_| tint);
        }
    }
}

/// Acrylic: soft shadow, saturated blurred backdrop, luminosity-graded tint, top sheen,
/// noise grain and a two-line rim light.
fn glass(p: &Painter, bg: &Bg, rect: Rect, rad: f32, k: f32) {
    for i in 0..6 {
        let g = (i + 1) as f32 * 2.0;
        p.add(Shape::convex_polygon(
            rounded_poly(rect.expand(g).translate(vec2(0.0, 4.0)), rad + g, 8),
            Color32::from_black_alpha((4.0 * k) as u8),
            Stroke::NONE,
        ));
    }
    let pts = rounded_poly(rect, rad, 10);
    tex_poly(p, bg, bg.blur, &pts, Color32::WHITE.gamma_multiply(k));
    let (t, b) = (rect.top(), rect.bottom());
    let h = (b - t).max(1.0);
    fan(p, egui::TextureId::default(), &pts, |_| egui::epaint::WHITE_UV, |q| {
        let f = ((q.y - t) / h).clamp(0.0, 1.0);
        Color32::from_white_alpha(((GLASS_A + 8.0 - 16.0 * f) * k) as u8)
    });
    fan(p, egui::TextureId::default(), &pts, |_| egui::epaint::WHITE_UV, |q| {
        let f = ((q.y - t) / (h * 0.5)).clamp(0.0, 1.0);
        Color32::from_white_alpha((16.0 * (1.0 - f) * k) as u8)
    });
    grain(p, bg, &pts, rect.min, (GRAIN_A as f32 * k) as u8);
    p.add(Shape::closed_line(pts, Stroke::new(1.0, Color32::from_white_alpha((120.0 * k) as u8))));
    p.add(Shape::closed_line(
        rounded_poly(rect.shrink(1.5), (rad - 1.5).max(0.0), 10),
        Stroke::new(1.0, Color32::from_white_alpha((30.0 * k) as u8)),
    ));
}

/// Pill slider: thin track, accent fill, white pill thumb.
fn pill_slider(ui: &mut Ui, rect: Rect, id: Id, v: &mut f32, lo: f32, hi: f32) -> bool {
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let act = resp.hovered() || resp.dragged();
    let h = ui.ctx().animate_bool_with_time(id.with("h"), act, 0.12);
    if act {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let thumb = vec2(46.0 + 3.0 * h, 26.0 + 1.0 * h);
    let (x0, x1) = (rect.left() + thumb.x * 0.5, rect.right() - thumb.x * 0.5);
    let mut changed = false;
    if resp.is_pointer_button_down_on() || resp.clicked() {
        if let Some(pp) = resp.interact_pointer_pos() {
            let t = ((pp.x - x0) / (x1 - x0)).clamp(0.0, 1.0);
            let nv = lo + t * (hi - lo);
            if (nv - *v).abs() > 1e-4 {
                *v = nv;
                changed = true;
            }
        }
    }
    let t = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
    let cx = x0 + t * (x1 - x0);
    let cy = rect.center().y;
    let p = ui.painter();
    let track = Rect::from_min_max(pos2(rect.left(), cy - 3.0), pos2(rect.right(), cy + 3.0));
    p.rect_filled(track, Rounding::same(3.0), Color32::from_black_alpha(36));
    p.rect_filled(
        Rect::from_min_max(track.min, pos2(cx, track.max.y)),
        Rounding::same(3.0),
        ACCENT,
    );
    let th = Rect::from_center_size(pos2(cx, cy), thumb);
    p.rect_filled(th.expand(1.0).translate(vec2(0.0, 1.5)), Rounding::same(14.0), Color32::from_black_alpha(34));
    p.rect_filled(th, Rounding::same(13.0), Color32::WHITE);
    changed
}

/// 5x7 pixel glyphs, just enough for the title.
fn glyph(c: char) -> [&'static str; 7] {
    match c {
        'K' => ["X...X", "X..X.", "X.X..", "XX...", "X.X..", "X..X.", "X...X"],
        'a' => [".....", ".....", ".XXX.", "....X", ".XXXX", "X...X", ".XXXX"],
        's' => [".....", ".....", ".XXXX", "X....", ".XXX.", "....X", "XXXX."],
        'n' => [".....", ".....", "X.XX.", "XX..X", "X...X", "X...X", "X...X"],
        'd' => ["....X", "....X", ".XXXX", "X...X", "X...X", "X...X", ".XXXX"],
        'r' => [".....", ".....", "X.XX.", "XX..X", "X....", "X....", "X...."],
        _ => ["....."; 7],
    }
}
fn pixel_text(p: &Painter, pos: Pos2, text: &str, px: f32, col: Color32) {
    let mut x = pos.x;
    for ch in text.chars() {
        for (ry, row) in glyph(ch).iter().enumerate() {
            for (rx, c) in row.chars().enumerate() {
                if c == 'X' {
                    let x0 = (x + rx as f32 * px).round();
                    let x1 = (x + (rx + 1) as f32 * px).round();
                    let y0 = (pos.y + ry as f32 * px).round();
                    let y1 = (pos.y + (ry + 1) as f32 * px).round();
                    p.rect_filled(Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)), Rounding::ZERO, col);
                }
            }
        }
        x += 6.0 * px;
    }
}

// ───────────────────────────── background ─────────────────────────────

/// Procedural version of the silver/periwinkle reeded-glass reference, lightly blurred.
fn procedural_bg() -> Vec<[f32; 3]> {
    const W: usize = BG_W;
    const H: usize = BG_H;
    let sm = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };
    let dark = [70.0, 78.0, 108.0];
    let light = [182.0, 194.0, 228.0];
    let mut buf = vec![[0f32; 3]; W * H];
    for y in 0..H {
        let v = y as f32 / (H - 1) as f32;
        for x in 0..W {
            let u = x as f32 / (W - 1) as f32;
            let mut c = mix(dark, light, sm(0.0, 0.95, u));
            let dx = u * 1.78;
            let d = (dx * dx + (v - 0.42) * (v - 0.42)).sqrt();
            c = mix(c, [52.0, 58.0, 88.0], (-(d / 0.5).powi(2)).exp() * 0.75);
            let d2 = (dx * dx + (v - 0.95) * (v - 0.95)).sqrt();
            c = mix(c, [176.0, 180.0, 214.0], (-(d2 / 0.55).powi(2)).exp() * 0.6);
            c = mix(c, [126.0, 136.0, 168.0], sm(0.7, 1.0, v) * sm(0.45, 1.0, u) * 0.55);

            let s = (v - 0.5) * 2.0;
            let bound = 0.50 + 0.44 * (1.0 - s * s);
            let k = sm(bound + 0.03, bound - 0.12, u) * sm(0.16, 0.24, u);
            if k > 0.0 {
                let p = u * 70.0 + 0.35 * (v * 5.0 + u * 4.0).sin();
                let rb = 0.45 + 0.55 * sm(0.35, 0.9, v);
                let mut out = c;
                for ch in 0..3 {
                    let off = [0.07, 0.0, -0.07][ch] * rb;
                    let f = (p + off).rem_euclid(1.0);
                    let band = 0.5 - 0.5 * (f * std::f32::consts::TAU).cos();
                    let hl = band.powi(4);
                    let shade = 1.0 - 0.30 * (1.0 - band);
                    let lit = 0.35 + 0.65 * sm(0.2, 0.9, u);
                    let target = [236.0, 242.0, 255.0][ch];
                    out[ch] = c[ch] * (1.0 - k * (1.0 - shade)) + k * hl * lit * (target - c[ch]) * 0.85;
                }
                c = out;
            }
            buf[y * W + x] = c;
        }
    }
    let mut buf = buf;
    box_blur_fast(&mut buf, W, H, 2);
    buf
}

/// Box blur in O(1) per pixel (prefix sums); edges average over the pixels that exist.
fn box_blur_fast(buf: &mut [[f32; 3]], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let mut tmp = buf.to_vec();
    let mut pre = vec![[0f32; 3]; w.max(h) + 1];
    for y in 0..h {
        for x in 0..w {
            let (p, q) = (buf[y * w + x], pre[x]);
            pre[x + 1] = [q[0] + p[0], q[1] + p[1], q[2] + p[2]];
        }
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r + 1).min(w));
            let n = (b - a) as f32;
            let (s1, s0) = (pre[b], pre[a]);
            tmp[y * w + x] = [(s1[0] - s0[0]) / n, (s1[1] - s0[1]) / n, (s1[2] - s0[2]) / n];
        }
    }
    for x in 0..w {
        for y in 0..h {
            let (p, q) = (tmp[y * w + x], pre[y]);
            pre[y + 1] = [q[0] + p[0], q[1] + p[1], q[2] + p[2]];
        }
        for y in 0..h {
            let (a, b) = (y.saturating_sub(r), (y + r + 1).min(h));
            let n = (b - a) as f32;
            let (s1, s0) = (pre[b], pre[a]);
            buf[y * w + x] = [(s1[0] - s0[0]) / n, (s1[1] - s0[1]) / n, (s1[2] - s0[2]) / n];
        }
    }
}

/// The wallpaper at blur radius `r` pixels (0 = untouched). Two box passes ~ a gaussian.
fn wall_buf(buf: &[[f32; 3]], r: usize) -> Vec<[f32; 3]> {
    let mut b = buf.to_vec();
    box_blur_fast(&mut b, BG_W, BG_H, r);
    box_blur_fast(&mut b, BG_W, BG_H, r);
    b
}

/// Quarter-res copy, used for the frosted glass layer.
fn downsample(buf: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let (sw, sh) = (BG_W / 4, BG_H / 4);
    let mut small = vec![[0f32; 3]; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            let mut s = [0f32; 3];
            for dy in 0..4 {
                for dx in 0..4 {
                    let p = buf[(y * 4 + dy) * BG_W + x * 4 + dx];
                    s[0] += p[0];
                    s[1] += p[1];
                    s[2] += p[2];
                }
            }
            small[y * sw + x] = [s[0] / 16.0, s[1] / 16.0, s[2] / 16.0];
        }
    }
    small
}

/// The frosted layer behind every glass surface.
fn glass_img(wall: &[[f32; 3]]) -> egui::ColorImage {
    let (sw, sh) = (BG_W / 4, BG_H / 4);
    let mut b = downsample(wall);
    box_blur(&mut b, sw, sh, 5);
    box_blur(&mut b, sw, sh, 5);
    for p in b.iter_mut() {
        let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        for c in p.iter_mut() {
            *c = l + (*c - l) * 1.25;
        }
    }
    to_img(&b, sw, sh)
}

/// Load any image, cover-crop it to the background aspect ratio.
fn load_bg(path: &Path) -> Result<Vec<[f32; 3]>, String> {
    let img = image::open(path).map_err(|e| e.to_string())?;
    let img = img.resize_to_fill(BG_W as u32, BG_H as u32, image::imageops::FilterType::Triangle).to_rgb8();
    Ok(img.pixels().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect())
}

/// egui's built-in font only has Latin glyphs, so append system fonts as fallbacks, and
/// register a "bold" family (real bold face when we can find one) for markdown headings / **bold**.
fn setup_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    let paths = [
        // Windows
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\seguisym.ttf",
        "C:\\Windows\\Fonts\\nirmala.ttf",
        "C:\\Windows\\Fonts\\malgun.ttf",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\YuGothR.ttc",
        // macOS
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/SFArabic.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        // Linux
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    for (i, path) in paths.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("sys{i}");
            fonts.font_data.insert(name.clone(), egui::FontData::from_owned(bytes));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&fam) {
                    list.push(name.clone());
                }
            }
        }
    }
    // bold family: real bold face first (if present), then everything the normal family has
    let mut bold = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    let bold_paths = [
        "C:\\Windows\\Fonts\\segoeuib.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ];
    for path in bold_paths {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert("boldface".to_string(), egui::FontData::from_owned(bytes));
            bold.insert(0, "boldface".to_string());
            break;
        }
    }
    fonts.families.insert(bold_family(), bold);
    ctx.set_fonts(fonts);
}

/// Decode the embedded kasandra.png at a given square size.
fn icon_rgba(size: u32) -> Option<image::RgbaImage> {
    image::load_from_memory(ICON_PNG)
        .ok()
        .map(|i| i.resize_exact(size, size, image::imageops::FilterType::Lanczos3).to_rgba8())
}

fn make_grain() -> egui::ColorImage {
    let mut st: u32 = 0x9E37_79B9;
    let mut bytes = Vec::with_capacity(256 * 256 * 3);
    for _ in 0..256 * 256 {
        st ^= st << 13;
        st ^= st >> 17;
        st ^= st << 5;
        let g = (st >> 24) as u8;
        bytes.extend_from_slice(&[g, g, g]);
    }
    egui::ColorImage::from_rgb([256, 256], &bytes)
}

fn to_img(buf: &[[f32; 3]], w: usize, h: usize) -> egui::ColorImage {
    let bytes: Vec<u8> = buf
        .iter()
        .flat_map(|p| [p[0].clamp(0.0, 255.0) as u8, p[1].clamp(0.0, 255.0) as u8, p[2].clamp(0.0, 255.0) as u8])
        .collect();
    egui::ColorImage::from_rgb([w, h], &bytes)
}

fn box_blur(buf: &mut Vec<[f32; 3]>, w: usize, h: usize, r: usize) {
    let mut tmp = buf.clone();
    for y in 0..h {
        for x in 0..w {
            let (mut s, mut n) = ([0f32; 3], 0f32);
            for dx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                let p = buf[y * w + dx];
                s[0] += p[0];
                s[1] += p[1];
                s[2] += p[2];
                n += 1.0;
            }
            tmp[y * w + x] = [s[0] / n, s[1] / n, s[2] / n];
        }
    }
    for y in 0..h {
        for x in 0..w {
            let (mut s, mut n) = ([0f32; 3], 0f32);
            for dy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                let p = tmp[dy * w + x];
                s[0] += p[0];
                s[1] += p[1];
                s[2] += p[2];
                n += 1.0;
            }
            buf[y * w + x] = [s[0] / n, s[1] / n, s[2] / n];
        }
    }
}

// ───────────────────────────── secure key storage + saved state ─────────────────────────────
// API keys live in the OS credential store (Windows Credential Manager / macOS Keychain /
// Secret Service). They are NEVER written to state.json.

fn key_account(p: &Provider) -> String {
    format!("{}|{}", p.name, p.base)
}
fn key_save(p: &Provider) -> Result<(), String> {
    keyring::Entry::new(KR_SERVICE, &key_account(p))
        .map_err(|e| e.to_string())?
        .set_password(&p.key)
        .map_err(|e| e.to_string())
}
fn key_load(p: &Provider) -> Option<String> {
    keyring::Entry::new(KR_SERVICE, &key_account(p)).ok()?.get_password().ok()
}
fn key_delete(p: &Provider) {
    if let Ok(e) = keyring::Entry::new(KR_SERVICE, &key_account(p)) {
        let _ = e.delete_credential();
    }
}

fn state_path() -> Option<PathBuf> {
    let d = dirs::config_dir()?.join("Kasandra");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("state.json"))
}

#[derive(Serialize)]
struct SavedRef<'a> {
    providers: &'a [Provider],
    active: &'a Option<(usize, String)>,
    projects: &'a [Project],
    chats: &'a [Chat],
    cur: usize,
    cur_project: Option<usize>,
    blur: f32,
    bg_path: &'a Option<String>,
}

#[derive(Deserialize, Default)]
struct Saved {
    #[serde(default)]
    providers: Vec<Provider>,
    #[serde(default)]
    active: Option<(usize, String)>,
    #[serde(default)]
    projects: Vec<Project>,
    #[serde(default)]
    chats: Vec<Chat>,
    #[serde(default)]
    cur: usize,
    #[serde(default)]
    cur_project: Option<usize>,
    #[serde(default)]
    blur: f32,
    #[serde(default)]
    bg_path: Option<String>,
}

fn load_state() -> Option<Saved> {
    let bytes = std::fs::read(state_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ───────────────────────────── providers / network ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
enum Kind {
    OpenAi,
    Anthropic,
    Gemini,
}

#[derive(Clone, Serialize, Deserialize)]
struct Provider {
    kind: Kind,
    name: String,
    base: String,
    #[serde(skip)] // never written to disk, lives in the OS keyring
    key: String,
    models: Vec<String>,
    #[serde(skip)]
    born: f64,
}

/// Every provider we know how to talk to: (kind, name, base url, key prefixes that identify it).
const PROVIDERS: &[(Kind, &str, &str, &[&str])] = &[
    (Kind::Anthropic, "Anthropic", "https://api.anthropic.com/v1", &["sk-ant-"]),
    (Kind::Gemini, "Google Gemini", "https://generativelanguage.googleapis.com/v1beta", &["AIza", "AQ."]),
    (Kind::OpenAi, "Groq", "https://api.groq.com/openai/v1", &["gsk_"]),
    (Kind::OpenAi, "xAI", "https://api.x.ai/v1", &["xai-"]),
    (Kind::OpenAi, "Cerebras", "https://api.cerebras.ai/v1", &["csk-"]),
    (Kind::OpenAi, "NVIDIA", "https://integrate.api.nvidia.com/v1", &["nvapi-"]),
    (Kind::OpenAi, "Fireworks", "https://api.fireworks.ai/inference/v1", &["fw_"]),
    (Kind::OpenAi, "OpenAI", "https://api.openai.com/v1", &["sk-proj-", "sk-svcacct-", "sk-"]),
    (Kind::OpenAi, "Mistral", "https://api.mistral.ai/v1", &[]),
    (Kind::OpenAi, "DeepSeek", "https://api.deepseek.com/v1", &["sk-"]),
    (Kind::OpenAi, "Together", "https://api.together.xyz/v1", &[]),
    (Kind::OpenAi, "SambaNova", "https://api.sambanova.ai/v1", &[]),
    (Kind::OpenAi, "Moonshot", "https://api.moonshot.ai/v1", &["sk-"]),
    (Kind::OpenAi, "Hugging Face", "https://router.huggingface.co/v1", &["hf_"]),
    (Kind::OpenAi, "OpenRouter", "https://openrouter.ai/api/v1", &["sk-or-"]),
];

/// Turn whatever was pasted into a clean key: strips quotes, "Bearer ", "export", and NAME=value wrappers.
fn clean_key(input: &str) -> String {
    let mut s = input.trim().trim_matches(|c: char| c == '"' || c == '\'' || c == '`').trim().to_string();
    if let Some(r) = s.strip_prefix("export ") {
        s = r.trim().to_string();
    }
    if let Some((name, val)) = s.split_once('=') {
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            s = val.trim().trim_matches(|c: char| c == '"' || c == '\'').to_string();
        }
    }
    if s.get(..7).map_or(false, |x| x.eq_ignore_ascii_case("bearer ")) {
        s = s[7..].trim().to_string();
    }
    s
}

/// Returns the providers to try, best guess first.
fn detect(input: &str) -> Result<(Vec<Provider>, bool), String> {
    let key = clean_key(input);
    if key.is_empty() {
        return Err("Paste an API key first".into());
    }
    let mk = |kind, name: &str, base: &str, key: &str| Provider {
        kind,
        name: name.to_string(),
        base: base.trim_end_matches('/').to_string(),
        key: key.to_string(),
        models: vec![],
        born: 0.0,
    };
    if let Some((b, k)) = key.split_once(char::is_whitespace) {
        if b.starts_with("http") {
            let host = b.split("//").nth(1).unwrap_or(b).split('/').next().unwrap_or(b);
            return Ok((vec![mk(Kind::OpenAi, host, b, k.trim())], true));
        }
        return Err("Expected just a key, or: https://host/v1 <key>".into());
    }
    let mut scored: Vec<(usize, usize)> = vec![];
    for (i, (_, _, _, prefixes)) in PROVIDERS.iter().enumerate() {
        if let Some(l) = prefixes.iter().filter(|p| key.starts_with(**p)).map(|p| p.len()).max() {
            scored.push((l, i));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let matched = !scored.is_empty();
    let mut order: Vec<usize> = scored.iter().map(|x| x.1).collect();
    for i in 0..PROVIDERS.len() {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    Ok((order.into_iter().map(|i| mk(PROVIDERS[i].0, PROVIDERS[i].1, PROVIDERS[i].2, &key)).collect(), matched))
}

/// Default model for a freshly added key.
fn pick_default(models: &[String]) -> Option<String> {
    let bad = ["allam", "whisper", "guard", "tts", "embed", "orpheus", "image", "vision-preview", "moderation", "aqa", "imagen", "veo"];
    let good = [
        "llama-3.3-70b", "gpt-4o", "gpt-4.1", "gpt-5", "claude-sonnet", "claude-3-5-sonnet", "gemini-2.5-flash",
        "gemini-2.5-pro", "gemini-2.0-flash", "deepseek-chat", "mistral-large", "grok", "llama", "gpt", "claude", "gemini",
    ];
    let ok: Vec<&String> = models.iter().filter(|m| !bad.iter().any(|b| m.to_lowercase().contains(b))).collect();
    for g in good {
        if let Some(m) = ok.iter().find(|m| m.to_lowercase().contains(g)) {
            return Some((*m).clone());
        }
    }
    ok.first().map(|m| (*m).clone()).or_else(|| models.first().cloned())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(120)).build()
}

/// Turns an HTTP error into "HTTP 503: <the provider's actual message>" instead of a raw JSON dump.
fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(c, r) => {
            let body = r.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v["error"]["message"]
                        .as_str()
                        .or_else(|| v["error"].as_str())
                        .or_else(|| v["message"].as_str())
                        .or_else(|| v[0]["error"]["message"].as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body.chars().take(200).collect());
            format!("HTTP {c}: {msg}")
        }
        e => e.to_string(),
    }
}
fn get_json(req: ureq::Request) -> Result<Value, String> {
    req.call().map_err(http_err)?.into_json::<Value>().map_err(|e| e.to_string())
}
fn post_json(req: ureq::Request, body: Value) -> Result<Value, String> {
    req.send_json(body).map_err(http_err)?.into_json::<Value>().map_err(|e| e.to_string())
}

fn fetch_models(p: &Provider) -> Result<Vec<String>, String> {
    let a = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(15)).build();
    let v = match p.kind {
        Kind::OpenAi => get_json(
            a.get(&format!("{}/models", p.base)).set("Authorization", &format!("Bearer {}", p.key)),
        )?,
        Kind::Anthropic => get_json(
            a.get(&format!("{}/models?limit=100", p.base))
                .set("x-api-key", &p.key)
                .set("anthropic-version", "2023-06-01"),
        )?,
        Kind::Gemini => get_json(
            a.get(&format!("{}/models?pageSize=200", p.base)).set("x-goog-api-key", &p.key),
        )?,
    };
    let mut out: Vec<String> = vec![];
    if p.kind == Kind::Gemini {
        for m in v["models"].as_array().into_iter().flatten() {
            let ok = m["supportedGenerationMethods"]
                .as_array()
                .map_or(true, |a| a.iter().any(|x| x.as_str() == Some("generateContent")));
            if ok {
                if let Some(n) = m["name"].as_str() {
                    out.push(n.trim_start_matches("models/").to_string());
                }
            }
        }
    } else {
        for m in v["data"].as_array().into_iter().flatten() {
            if let Some(id) = m["id"].as_str() {
                out.push(id.to_string());
            }
        }
    }
    let skip = [
        "allam", "arabic", "orpheus", "whisper", "tts", "speech", "audio", "guard", "embed", "moderation", "rerank",
        "imagen", "image", "veo", "aqa", "transcribe", "realtime", "dall-e", "playai",
    ];
    out.retain(|m| {
        let l = m.to_lowercase();
        !skip.iter().any(|w| l.contains(w))
    });
    out.sort();
    out.dedup();
    if out.is_empty() {
        Err("No models returned for this key".into())
    } else {
        Ok(out)
    }
}

/// Sent with every request so every model, from every provider, answers in English only.
const SYSTEM_PROMPT: &str = "You must reply in English only, no matter what language the user writes in. Never use Arabic or any other non-English language in your replies. Format answers in clean Markdown: short paragraphs, headings for sections, bullet or numbered lists, tables when comparing things, and fenced code blocks with a language tag for code.";

fn chat_once(p: &Provider, model: &str, h: &[(bool, String)], extra: &str) -> Result<String, String> {
    let a = agent();
    let empty = || "Empty response".to_string();
    let system = format!("{SYSTEM_PROMPT}{extra}");
    match p.kind {
        Kind::OpenAi => {
            let mut msgs: Vec<Value> = vec![json!({"role": "system", "content": system})];
            msgs.extend(h.iter().map(|(u, t)| json!({"role": if *u {"user"} else {"assistant"}, "content": t})));
            let v = post_json(
                a.post(&format!("{}/chat/completions", p.base))
                    .set("Authorization", &format!("Bearer {}", p.key)),
                json!({"model": model, "messages": msgs}),
            )?;
            v["choices"][0]["message"]["content"].as_str().map(str::to_string).ok_or_else(empty)
        }
        Kind::Anthropic => {
            let msgs: Vec<Value> = h
                .iter()
                .map(|(u, t)| json!({"role": if *u {"user"} else {"assistant"}, "content": t}))
                .collect();
            let v = post_json(
                a.post(&format!("{}/messages", p.base))
                    .set("x-api-key", &p.key)
                    .set("anthropic-version", "2023-06-01"),
                json!({"model": model, "max_tokens": 4096, "system": system, "messages": msgs}),
            )?;
            v["content"][0]["text"].as_str().map(str::to_string).ok_or_else(empty)
        }
        Kind::Gemini => {
            let contents: Vec<Value> = h
                .iter()
                .map(|(u, t)| json!({"role": if *u {"user"} else {"model"}, "parts": [{"text": t}]}))
                .collect();
            let v = post_json(
                a.post(&format!("{}/models/{}:generateContent", p.base, model))
                    .set("x-goog-api-key", &p.key),
                json!({ "systemInstruction": {"parts": [{"text": system}]}, "contents": contents }),
            )?;
            v["candidates"][0]["content"]["parts"][0]["text"].as_str().map(str::to_string).ok_or_else(empty)
        }
    }
}

/// Retries "high demand" (503) and rate-limit (429) errors a couple of times before giving up.
fn chat_call(p: &Provider, model: &str, h: &[(bool, String)], extra: &str) -> Result<String, String> {
    let mut attempt = 0;
    loop {
        match chat_once(p, model, h, extra) {
            Err(e) if attempt < 2 && (e.starts_with("HTTP 503") || e.starts_with("HTTP 429")) => {
                attempt += 1;
                std::thread::sleep(Duration::from_secs(2 * attempt));
            }
            r => return r,
        }
    }
}

// ───────────────────────────── project folder access ─────────────────────────────
// Models can't touch your disk by themselves. Instead, on every message we read the project
// folder on a worker thread and hand the model a fresh read-only snapshot: the file tree plus the
// contents of the most relevant text files (files you mention by name come first).

const CTX_BUDGET: usize = 40_000; // total characters of file contents per request (~10k tokens)
const FILE_CAP: usize = 12_000; // per-file cap (files you name explicitly get more)
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "out", "venv", "__pycache__", "bin", "obj", "vendor", "coverage",
];
const TEXT_EXT: &[&str] = &[
    "rs", "toml", "md", "txt", "py", "js", "mjs", "ts", "tsx", "jsx", "json", "html", "htm", "css", "scss", "c", "h",
    "cpp", "hpp", "cs", "java", "kt", "go", "rb", "php", "sh", "bat", "ps1", "yml", "yaml", "xml", "sql", "lua",
    "swift", "gradle", "ini", "cfg", "csv", "vue", "svelte", "dart",
];
const KEY_FILES: &[&str] = &[
    "readme.md", "cargo.toml", "package.json", "main.rs", "lib.rs", "main.py", "app.py", "index.js", "index.html",
    "main.ts", "pyproject.toml", "main.go", "program.cs",
];

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf, u64)>) {
    if depth > 6 || out.len() > 1500 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut es: Vec<_> = rd.flatten().collect();
    es.sort_by_key(|e| e.file_name());
    for e in es {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if name.starts_with('.') || SKIP_DIRS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            walk(root, &path, depth + 1, out);
        } else if ft.is_file() {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            out.push((rel, path, size));
        }
    }
}

fn human(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / 1048576.0)
    }
}

fn project_context(path: &str, query: &str) -> String {
    let root = Path::new(path);
    if !root.is_dir() {
        return format!("\n\nThe user attached a project folder at {path}, but it no longer exists or can't be read.");
    }
    let mut files = vec![];
    walk(root, root, 0, &mut files);
    if files.is_empty() {
        return format!("\n\nThe user attached a project folder at {path}. It is empty (or contains nothing readable).");
    }
    let q = query.to_lowercase();
    let mut cand: Vec<(u8, u64, usize)> = vec![];
    for (i, (rel, _, size)) in files.iter().enumerate() {
        let name = rel.rsplit('/').next().unwrap_or(rel).to_lowercase();
        if name.starts_with('.') || name.ends_with(".lock") || name == "package-lock.json" {
            continue;
        }
        let ext = Path::new(rel).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let mentioned = name.len() >= 3 && (q.contains(&name) || q.contains(&rel.to_lowercase()));
        let texty = TEXT_EXT.contains(&ext.as_str()) || matches!(name.as_str(), "readme" | "makefile" | "dockerfile");
        if mentioned && *size <= 300_000 {
            cand.push((0, *size, i));
        } else if texty && *size <= 80_000 {
            cand.push((if KEY_FILES.contains(&name.as_str()) { 1 } else { 2 }, *size, i));
        }
    }
    cand.sort();

    let mut s = format!(
        "\n\nThe user attached a project folder: {path}\n\
You CAN see it: below is a live, read-only snapshot taken just now (file tree, then file contents). \
Answer using it, and refer to files by their relative path. You cannot modify files, so present any changes as code blocks. \
If a file you need was cut off or isn't included, ask the user to name it.\n\n## File tree\n"
    );
    for (rel, _, size) in files.iter().take(400) {
        s.push_str(&format!("{rel} ({})\n", human(*size)));
    }
    if files.len() > 400 {
        s.push_str(&format!("... and {} more files\n", files.len() - 400));
    }
    s.push_str("\n## File contents\n");
    let mut used = 0usize;
    for (rank, _, i) in cand {
        if used >= CTX_BUDGET {
            break;
        }
        let (rel, abs, _) = &files[i];
        let Ok(txt) = std::fs::read_to_string(abs) else { continue };
        let cap = if rank == 0 { 30_000 } else { FILE_CAP };
        let take = cap.min(CTX_BUDGET - used);
        let body: String = txt.chars().take(take).collect();
        let cut = txt.chars().count() > take;
        s.push_str(&format!("\n=== {rel} ===\n{body}{}\n", if cut { "\n[... truncated]" } else { "" }));
        used += body.len();
    }
    s
}

// ───────────────────────────── markdown rendering ─────────────────────────────

/// Inline markdown (**bold**, *italic*, `code`) -> one wrapped LayoutJob.
fn inline_job(text: &str, size: f32, col: Color32, bold: bool) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    let body = FontId::proportional(size);
    let boldf = FontId::new(size, bold_family());
    let mono = FontId::monospace(size - 1.5);
    let lh = Some(size * 1.55);
    let chars: Vec<char> = text.chars().collect();
    let (mut b, mut it, mut i) = (bold, false, 0usize);
    let mut buf = String::new();
    let flush = |job: &mut LayoutJob, buf: &mut String, b: bool, it: bool| {
        if buf.is_empty() {
            return;
        }
        job.append(
            buf,
            0.0,
            TextFormat {
                font_id: if b { boldf.clone() } else { body.clone() },
                color: col,
                italics: it,
                line_height: lh,
                ..Default::default()
            },
        );
        buf.clear();
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            if let Some(j) = chars[i + 1..].iter().position(|&x| x == '`') {
                flush(&mut job, &mut buf, b, it);
                let code: String = chars[i + 1..i + 1 + j].iter().collect();
                job.append(
                    &code,
                    0.0,
                    TextFormat {
                        font_id: mono.clone(),
                        color: Color32::from_rgb(150, 40, 90),
                        background: Color32::from_black_alpha(22),
                        line_height: lh,
                        ..Default::default()
                    },
                );
                i += j + 2;
                continue;
            }
        }
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            flush(&mut job, &mut buf, b, it);
            b = !b;
            i += 2;
            continue;
        }
        if c == '*'
            && (it || (chars.get(i + 1).map_or(false, |x| !x.is_whitespace()) && chars[i + 1..].contains(&'*')))
        {
            flush(&mut job, &mut buf, b, it);
            it = !it;
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut job, &mut buf, b, it);
    job
}

fn code_block(ui: &mut Ui, lang: &str, code: &str, id: Id) {
    let w = (ui.available_width() - 24.0).max(100.0);
    egui::Frame::none()
        .fill(Color32::from_rgb(28, 33, 48))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_min_width(w);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if lang.is_empty() { "code" } else { lang })
                        .size(11.5)
                        .color(Color32::from_rgb(140, 152, 180)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let now = ui.input(|i| i.time);
                    let at: f64 = ui.data(|d| d.get_temp::<f64>(id.with("cp"))).unwrap_or(-10.0);
                    let copied = now - at < 1.5;
                    if copied {
                        ui.ctx().request_repaint_after(Duration::from_millis(1600));
                    }
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(if copied { "Copied" } else { "Copy" })
                                .size(11.5)
                                .color(Color32::from_rgb(190, 200, 225)),
                        )
                        .sense(Sense::click()),
                    );
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                    if r.clicked() {
                        ui.output_mut(|o| o.copied_text = code.to_string());
                        ui.data_mut(|d| d.insert_temp(id.with("cp"), now));
                    }
                });
            });
            ui.add_space(4.0);
            egui::ScrollArea::horizontal().id_source(id.with("sc")).auto_shrink([false, true]).show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(code).monospace().size(13.0).color(Color32::from_rgb(222, 230, 247)),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
        });
}

fn list_item(ui: &mut Ui, level: usize, marker: Option<&str>, text: &str, col: Color32) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_space(4.0 + level as f32 * 8.0);
        let (r, _) = ui.allocate_exact_size(vec2(22.0, LINE_H), Sense::hover());
        let cy = r.top() + LINE_H / 2.0;
        match marker {
            Some(m) => {
                ui.painter().text(
                    pos2(r.right() - 2.0, cy),
                    Align2::RIGHT_CENTER,
                    m,
                    FontId::new(BODY_SIZE, bold_family()),
                    GREEN_DK,
                );
            }
            None => {
                ui.painter().circle_filled(pos2(r.left() + 10.0, cy), 2.6, GREEN_DK);
            }
        }
        ui.add(egui::Label::new(inline_job(text, BODY_SIZE, col, false)).wrap_mode(egui::TextWrapMode::Wrap));
    });
}

/// Block-level markdown: headings, paragraphs, bullet/numbered lists, quotes, tables, rules, code fences.
fn md_ui(ui: &mut Ui, text: &str, col: Color32, salt: Id) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let tr = line.trim_start();

        // fenced code block
        if tr.starts_with("```") {
            let lang = tr[3..].trim().to_string();
            let mut code: Vec<&str> = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            i += 1; // closing fence
            ui.add_space(4.0);
            code_block(ui, &lang, &code.join("\n"), salt.with(("code", i)));
            ui.add_space(6.0);
            continue;
        }

        if tr.is_empty() {
            ui.add_space(6.0);
            i += 1;
            continue;
        }

        // horizontal rule
        if tr.len() >= 3 && (tr.chars().all(|c| c == '-') || tr.chars().all(|c| c == '*') || tr.chars().all(|c| c == '_')) {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
            ui.painter().line_segment(
                [pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)],
                Stroke::new(1.0, Color32::from_black_alpha(34)),
            );
            i += 1;
            continue;
        }

        // heading
        let hashes = tr.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && tr[hashes..].starts_with(' ') {
            let size = match hashes {
                1 => 22.0,
                2 => 19.5,
                3 => 17.0,
                _ => 15.5,
            };
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(inline_job(tr[hashes..].trim(), size, col, true)).wrap_mode(egui::TextWrapMode::Wrap),
            );
            ui.add_space(2.0);
            i += 1;
            continue;
        }

        // table
        if tr.starts_with('|') {
            let mut rows: Vec<Vec<String>> = vec![];
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                let cells: Vec<String> =
                    lines[i].trim().trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
                let sep = cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' '));
                if !sep {
                    rows.push(cells);
                }
                i += 1;
            }
            let gid = salt.with(("tbl", i));
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(Color32::from_black_alpha(10))
                .rounding(Rounding::same(10.0))
                .inner_margin(Margin::symmetric(12.0, 8.0))
                .show(ui, |ui| {
                    egui::ScrollArea::horizontal().id_source(gid.with("s")).auto_shrink([false, true]).show(ui, |ui| {
                        egui::Grid::new(gid).spacing([18.0, 8.0]).show(ui, |ui| {
                            for (ri, r) in rows.iter().enumerate() {
                                for c in r {
                                    ui.add(
                                        egui::Label::new(inline_job(c, 14.0, col, ri == 0))
                                            .wrap_mode(egui::TextWrapMode::Wrap),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                    });
                });
            ui.add_space(6.0);
            continue;
        }

        // blockquote
        if let Some(rest) = tr.strip_prefix('>') {
            egui::Frame::none()
                .fill(Color32::from_black_alpha(12))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(12.0, 6.0))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(inline_job(rest.trim(), BODY_SIZE, fa(col, 0.75), false))
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                });
            i += 1;
            continue;
        }

        // lists
        let level = (line.len() - tr.len()) / 2;
        let mut handled = false;
        for m in ["- ", "* ", "+ ", "• "] {
            if let Some(rest) = tr.strip_prefix(m) {
                list_item(ui, level, None, rest.trim(), col);
                handled = true;
                break;
            }
        }
        if handled {
            i += 1;
            continue;
        }
        let dig = tr.chars().take_while(|c| c.is_ascii_digit()).count();
        if dig > 0 && dig <= 3 && (tr[dig..].starts_with(". ") || tr[dig..].starts_with(") ")) {
            let marker = format!("{}.", &tr[..dig]);
            list_item(ui, level, Some(&marker), tr[dig + 2..].trim(), col);
            i += 1;
            continue;
        }

        // paragraph
        ui.add(egui::Label::new(inline_job(tr, BODY_SIZE, col, false)).wrap_mode(egui::TextWrapMode::Wrap));
        i += 1;
    }
}

// ───────────────────────────── app state ─────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
struct Msg {
    user: bool,
    text: String,
    #[serde(default)]
    meta: String, // "Provider · model" shown under assistant replies
    #[serde(default)]
    err: bool,
}
#[derive(Serialize, Deserialize)]
struct Chat {
    #[serde(default)]
    id: u64,
    title: String,
    project: Option<usize>,
    msgs: Vec<Msg>,
    #[serde(skip)]
    pending: bool,
}
impl Chat {
    fn new(id: u64, project: Option<usize>) -> Self {
        Chat { id, title: "New chat".into(), project, msgs: vec![], pending: false }
    }
}
#[derive(Serialize, Deserialize)]
struct Project {
    name: String,
    path: String,
    #[serde(default)]
    open: bool,
}
enum Ev {
    Added(Result<Provider, String>),
    Reply(u64, String, Result<String, String>),
}
#[derive(Clone, Copy)]
enum Del {
    Chat(usize),
    Project(usize),
}
enum Act {
    NewChat(usize),
    Select(usize),
    NewProject,
    ToggleProject(usize),
    PickBg,
    AskDelete(Del),
}

struct App {
    bg: TextureHandle,
    bg_blur: TextureHandle,
    grain: TextureHandle,
    icon: TextureHandle,
    bg_buf: Vec<[f32; 3]>,
    bg_path: Option<String>,
    blur_amt: f32,
    blur_applied: usize,
    sidebar: bool,
    popup: bool,
    chats: Vec<Chat>,
    next_id: u64,
    cur: usize,
    projects: Vec<Project>,
    cur_project: Option<usize>,
    providers: Vec<Provider>,
    active: Option<(usize, String)>,
    key_input: String,
    adding: bool,
    add_err: Option<String>,
    input: String,
    confirm: Option<Del>,
    confirm_text: (String, String),
    dirty: bool,
    last_save: f64,
    tx: Sender<Ev>,
    rx: Receiver<Ev>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let mut v = egui::Visuals::light();
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.fg_stroke.color = INK;
        }
        v.selection.bg_fill = GREEN.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, GREEN);
        v.window_stroke = Stroke::NONE;
        cc.egui_ctx.set_visuals(v);

        // ── restore saved state (keys come back from the OS keyring) ──
        let saved = load_state().unwrap_or_default();
        let mut map: Vec<Option<usize>> = vec![];
        let mut providers: Vec<Provider> = vec![];
        for mut p in saved.providers {
            match key_load(&p) {
                Some(k) => {
                    p.key = k;
                    map.push(Some(providers.len()));
                    providers.push(p);
                }
                None => map.push(None), // key is gone from the keyring: drop the provider
            }
        }
        let active = saved.active.and_then(|(i, m)| map.get(i).copied().flatten().map(|ni| (ni, m)));
        let projects = saved.projects;
        let mut chats = saved.chats;
        for (i, c) in chats.iter_mut().enumerate() {
            c.id = i as u64 + 1;
            c.pending = false;
            if c.project.map_or(false, |p| p >= projects.len()) {
                c.project = None;
            }
        }
        let mut next_id = chats.len() as u64 + 1;
        if chats.is_empty() {
            chats.push(Chat::new(next_id, None));
            next_id += 1;
        }
        let cur = saved.cur.min(chats.len() - 1);
        let cur_project = saved.cur_project.filter(|p| *p < projects.len());
        let blur_amt = saved.blur.clamp(0.0, BLUR_MAX);

        let mut bg_path = saved.bg_path;
        let bg_buf = match bg_path.as_deref().map(|p| load_bg(Path::new(p))) {
            Some(Ok(b)) => b,
            _ => {
                bg_path = None;
                procedural_bg()
            }
        };
        let r = blur_r(blur_amt);
        let wall = wall_buf(&bg_buf, r);
        let bg = cc.egui_ctx.load_texture("bg", to_img(&wall, BG_W, BG_H), egui::TextureOptions::LINEAR);
        let bg_blur = cc.egui_ctx.load_texture("bg_blur", glass_img(&wall), egui::TextureOptions::LINEAR);
        let icon_img = match icon_rgba(64) {
            Some(im) => egui::ColorImage::from_rgba_unmultiplied([64, 64], &im.into_raw()),
            None => egui::ColorImage::new([1, 1], Color32::TRANSPARENT),
        };
        let icon = cc.egui_ctx.load_texture("icon", icon_img, egui::TextureOptions::LINEAR);
        let grain = cc.egui_ctx.load_texture("grain", make_grain(), egui::TextureOptions::NEAREST);
        let (tx, rx) = channel();
        Self {
            bg,
            bg_blur,
            grain,
            icon,
            bg_buf,
            bg_path,
            blur_amt,
            blur_applied: r,
            sidebar: false,
            popup: false,
            chats,
            next_id,
            cur,
            projects,
            cur_project,
            providers,
            active,
            key_input: String::new(),
            adding: false,
            add_err: None,
            input: String::new(),
            confirm: None,
            confirm_text: (String::new(), String::new()),
            dirty: false,
            last_save: 0.0,
            tx,
            rx,
        }
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ───────────── persistence ─────────────
    fn save_state(&self) {
        let Some(path) = state_path() else { return };
        let s = SavedRef {
            providers: &self.providers,
            active: &self.active,
            projects: &self.projects,
            chats: &self.chats,
            cur: self.cur,
            cur_project: self.cur_project,
            blur: self.blur_amt,
            bg_path: &self.bg_path,
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&s) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    fn flush(&mut self, ctx: &Context) {
        let closing = ctx.input(|i| i.viewport().close_requested());
        if !self.dirty && !closing {
            return;
        }
        let now = ctx.input(|i| i.time);
        if closing || now - self.last_save > 0.6 {
            self.save_state();
            self.dirty = false;
            self.last_save = now;
        } else {
            ctx.request_repaint_after(Duration::from_millis(700));
        }
    }

    fn poll(&mut self, ctx: &Context) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Ev::Added(Ok(mut p)) => {
                    self.adding = false;
                    self.key_input.clear();
                    p.born = ctx.input(|i| i.time);
                    let saved = key_save(&p);
                    let first = pick_default(&p.models);
                    let idx = if let Some(i) = self.providers.iter().position(|q| q.name == p.name && q.base == p.base) {
                        self.providers[i] = p;
                        i
                    } else {
                        self.providers.push(p);
                        self.providers.len() - 1
                    };
                    if self.active.is_none() {
                        if let Some(m) = first {
                            self.active = Some((idx, m));
                        }
                    }
                    if let Err(e) = saved {
                        self.add_err = Some(format!("Added, but the key couldn't be saved securely: {e}"));
                    }
                    self.dirty = true;
                }
                Ev::Added(Err(e)) => {
                    self.adding = false;
                    self.add_err = Some(e);
                }
                Ev::Reply(id, meta, r) => {
                    if let Some(c) = self.chats.iter_mut().find(|c| c.id == id) {
                        c.pending = false;
                        let (text, err) = match r {
                            Ok(t) => (t, false),
                            Err(e) => (e, true),
                        };
                        c.msgs.push(Msg { user: false, text, meta, err });
                        self.dirty = true;
                    }
                }
            }
        }
    }

    fn apply_blur(&mut self) {
        let r = blur_r(self.blur_amt);
        let wall = wall_buf(&self.bg_buf, r);
        self.bg.set(to_img(&wall, BG_W, BG_H), egui::TextureOptions::LINEAR);
        self.bg_blur.set(glass_img(&wall), egui::TextureOptions::LINEAR);
        self.blur_applied = r;
    }

    fn set_bg(&mut self, buf: Vec<[f32; 3]>) {
        self.bg_buf = buf;
        self.apply_blur();
    }

    /// New chat inside project `pj` (re-uses an empty one if it exists).
    fn new_chat(&mut self, pj: usize) {
        self.projects[pj].open = true;
        self.cur_project = Some(pj);
        if let Some(i) = self.chats.iter().position(|c| c.msgs.is_empty() && c.project == Some(pj)) {
            self.cur = i;
            return;
        }
        let id = self.fresh_id();
        self.chats.push(Chat::new(id, Some(pj)));
        self.cur = self.chats.len() - 1;
        self.dirty = true;
    }

    fn delete_chat(&mut self, ci: usize) {
        if ci >= self.chats.len() {
            return;
        }
        self.chats.remove(ci);
        if self.chats.is_empty() {
            let id = self.fresh_id();
            self.chats.push(Chat::new(id, None));
            self.cur = 0;
        } else if self.cur > ci {
            self.cur -= 1;
        } else if self.cur >= self.chats.len() || self.cur == ci {
            self.cur = ci.min(self.chats.len() - 1);
        }
        self.dirty = true;
    }

    fn delete_project(&mut self, pi: usize) {
        if pi >= self.projects.len() {
            return;
        }
        let cur_id = self.chats.get(self.cur).map(|c| c.id);
        self.chats.retain(|c| c.project != Some(pi));
        for c in self.chats.iter_mut() {
            if let Some(p) = c.project {
                if p > pi {
                    c.project = Some(p - 1);
                }
            }
        }
        self.projects.remove(pi);
        self.cur_project = match self.cur_project {
            Some(p) if p == pi => None,
            Some(p) if p > pi => Some(p - 1),
            o => o,
        };
        if self.chats.is_empty() {
            let id = self.fresh_id();
            self.chats.push(Chat::new(id, None));
        }
        self.cur = cur_id.and_then(|id| self.chats.iter().position(|c| c.id == id)).unwrap_or(0);
        self.dirty = true;
    }

    fn send(&mut self, ctx: &Context) {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.chats[self.cur].pending {
            return;
        }
        let Some((pi, model)) = self.active.clone() else {
            self.popup = true;
            return;
        };
        let p = self.providers[pi].clone();
        let meta = format!("{} · {}", p.name, model);
        let cur_project = self.cur_project;
        let chat = &mut self.chats[self.cur];
        if chat.msgs.is_empty() {
            chat.title = trunc(&text, 28);
            if chat.project.is_none() {
                chat.project = cur_project;
            }
        }
        chat.msgs.push(Msg { user: true, text: text.clone(), meta: String::new(), err: false });
        chat.pending = true;
        let chat_id = chat.id;
        let proj_path = chat.project.and_then(|i| self.projects.get(i)).map(|p| p.path.clone());
        let hist: Vec<(bool, String)> = chat.msgs.iter().map(|m| (m.user, m.text.clone())).collect();
        self.input.clear();
        self.dirty = true;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            // fresh read-only snapshot of the project folder on every message
            let extra = proj_path.map(|path| project_context(&path, &text)).unwrap_or_default();
            let r = chat_call(&p, &model, &hist, &extra);
            let _ = tx.send(Ev::Reply(chat_id, meta, r));
            ctx.request_repaint();
        });
    }

    fn start_add(&mut self, ctx: &Context) {
        if self.adding {
            return;
        }
        match detect(&self.key_input) {
            Err(e) => self.add_err = Some(e),
            Ok((cands, matched)) => {
                self.adding = true;
                self.add_err = None;
                let (tx, ctx) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let n = cands.len();
                    let mut first_err: Option<String> = None;
                    let mut found: Option<Provider> = None;
                    for p in cands {
                        match fetch_models(&p) {
                            Ok(m) if !m.is_empty() => {
                                found = Some(Provider { models: m, ..p });
                                break;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                if first_err.is_none() {
                                    first_err = Some(e);
                                }
                            }
                        }
                    }
                    let r = match found {
                        Some(p) => Ok(p),
                        None if matched && n > 1 => Err(first_err.unwrap_or_else(|| "Key was rejected".into())),
                        None if n == 1 => Err(first_err.unwrap_or_else(|| "Key was rejected".into())),
                        None => Err(format!(
                            "No known provider accepted this key (tried {n}). For others paste: https://host/v1 <key>"
                        )),
                    };
                    let _ = tx.send(Ev::Added(r));
                    ctx.request_repaint();
                });
            }
        }
    }

    // ───────────── title bar ─────────────
    fn titlebar(&mut self, ctx: &Context) {
        let screen = ctx.screen_rect();
        let rect = Rect::from_min_size(screen.min, vec2(screen.width(), TITLE_H));
        let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        Area::new(Id::new("titlebar")).order(Order::Foreground).fixed_pos(rect.min).constrain(false).show(
            ctx,
            |ui| {
                ui.set_min_size(rect.size());
                let drag = ui.interact(rect, Id::new("tb_drag"), Sense::click_and_drag());
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
                }
                let p = ui.painter().clone();
                let px = 2.5;
                let ty = rect.center().y - 3.5 * px;
                let ir = Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), vec2(22.0, 22.0));
                p.image(self.icon.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                pixel_text(&p, pos2(rect.left() + 46.0, ty + 1.0), "Kasandra", px, Color32::from_black_alpha(70));
                pixel_text(&p, pos2(rect.left() + 46.0, ty), "Kasandra", px, Color32::from_rgb(244, 247, 255));

                let cy = rect.center().y;
                let xs = [rect.right() - 68.0, rect.right() - 44.0, rect.right() - 20.0];
                let cols = [Color32::from_rgb(254, 188, 46), Color32::from_rgb(40, 200, 64), Color32::from_rgb(255, 95, 87)];
                let group = Rect::from_min_max(pos2(xs[0] - 12.0, cy - 12.0), pos2(xs[2] + 12.0, cy + 12.0));
                let gh = ctx.animate_bool_with_time(
                    Id::new("orbs_h"),
                    ctx.pointer_hover_pos().map_or(false, |q| group.contains(q)),
                    0.07,
                );
                for (i, cx) in xs.iter().enumerate() {
                    let c = pos2(*cx, cy);
                    let (r, h) = hit(ui, Rect::from_center_size(c, vec2(20.0, 20.0)), Id::new(("orb", i)));
                    let rad = 7.0 + h * 0.8;
                    let col = cols[i];
                    p.circle_filled(c, rad, col);
                    p.circle_stroke(c, rad, Stroke::new(1.0, lerpc(col, Color32::WHITE, 0.0).gamma_multiply(0.6)));
                    if gh > 0.01 {
                        let g = lerpc(col, INK, 0.75).gamma_multiply(gh);
                        let s = Stroke::new(1.5, g);
                        match i {
                            0 => {
                                p.line_segment([c + vec2(-3.0, 0.0), c + vec2(3.0, 0.0)], s);
                            }
                            1 => {
                                p.line_segment([c + vec2(-3.0, 0.0), c + vec2(3.0, 0.0)], s);
                                p.line_segment([c + vec2(0.0, -3.0), c + vec2(0.0, 3.0)], s);
                            }
                            _ => {
                                p.line_segment([c + vec2(-2.6, -2.6), c + vec2(2.6, 2.6)], s);
                                p.line_segment([c + vec2(-2.6, 2.6), c + vec2(2.6, -2.6)], s);
                            }
                        }
                    }
                    if r.clicked() {
                        ctx.send_viewport_cmd(match i {
                            0 => egui::ViewportCommand::Minimized(true),
                            1 => egui::ViewportCommand::Maximized(!maxed),
                            _ => egui::ViewportCommand::Close,
                        });
                    }
                }
            },
        );
    }

    // ───────────── hamburger ─────────────
    fn burger(&mut self, ctx: &Context, t: f32) {
        let rect = Rect::from_min_size(pos2(14.0, TITLE_H + 12.0), vec2(36.0, 36.0));
        let mut toggle = false;
        Area::new(Id::new("burger")).order(Order::Tooltip).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
            ui.set_min_size(rect.size());
            let (r, h) = hit(ui, rect, Id::new("burger_hit"));
            let p = ui.painter().clone();
            p.circle_filled(rect.center(), 18.0, Color32::from_white_alpha((h * 70.0) as u8));
            let c = rect.center();
            let col = lerpc(Color32::WHITE, INK, t);
            let half = 9.0;
            let line = |center: Pos2, ang: f32, a: f32| {
                let d = vec2(ang.cos(), ang.sin()) * half;
                p.line_segment([center - d, center + d], Stroke::new(2.0, col.gamma_multiply(a)));
            };
            let q = std::f32::consts::FRAC_PI_4;
            line(c + vec2(0.0, -6.0 * (1.0 - t)), t * q, 1.0);
            line(c, 0.0, 1.0 - t);
            line(c + vec2(0.0, 6.0 * (1.0 - t)), -t * q, 1.0);
            toggle = r.clicked();
        });
        if toggle {
            self.sidebar = !self.sidebar;
        }
    }

    // ───────────── sidebar: projects (parents) → chats (children) ─────────────
    fn sidebar_ui(&mut self, ctx: &Context, t: f32, bg: &Bg) {
        if t < 0.002 {
            return;
        }
        let screen = ctx.screen_rect();
        let h = screen.height() - TITLE_H - 16.0;
        let x = 12.0 - (SIDEBAR_W + 24.0) * (1.0 - t);
        let rect = Rect::from_min_size(pos2(x, TITLE_H + 8.0), vec2(SIDEBAR_W, h));
        let mut acts: Vec<Act> = vec![];
        let mut blur_v = self.blur_amt;
        Area::new(Id::new("sidebar")).order(Order::Foreground).fixed_pos(rect.min).constrain(false).show(
            ctx,
            |ui| {
                ui.set_min_size(rect.size());
                glass(ui.painter(), bg, rect, PANEL_R, 1.0);
                let inner = rect.shrink2(vec2(14.0, 12.0));
                let list = Rect::from_min_max(inner.min, pos2(inner.max.x, inner.max.y - BOT_H));
                ui.allocate_ui_at_rect(list, |ui| {
                    ui.add_space(46.0);
                    if section_header(ui, "Projects", Id::new("sh_proj")) {
                        acts.push(Act::NewProject);
                    }
                    egui::ScrollArea::vertical()
                        .max_height(list.height() - 46.0 - 28.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            if self.projects.is_empty() {
                                ui.label(RichText::new("  Press + to add a project folder").size(12.0).color(muted()));
                            }
                            for (pi, pr) in self.projects.iter().enumerate() {
                                let e = ease(ui.ctx().animate_bool_with_time(Id::new(("pe", pi)), pr.open, 0.14));
                                let w = ui.available_width();
                                let (row, _) = ui.allocate_exact_size(vec2(w, PROJ_ROW), Sense::hover());
                                let p = ui.painter().clone();
                                let o = side_row_at(
                                    ui, &p, row, row, Id::new(("proj", pi)), &pr.name, Some(&pr.path),
                                    self.cur_project == Some(pi), 0.0, true, Some(e),
                                );
                                if o.clicked {
                                    acts.push(Act::ToggleProject(pi));
                                }
                                if o.plus {
                                    acts.push(Act::NewChat(pi));
                                }
                                if o.delete {
                                    acts.push(Act::AskDelete(Del::Project(pi)));
                                }
                                let kids: Vec<usize> =
                                    (0..self.chats.len()).rev().filter(|&ci| self.chats[ci].project == Some(pi)).collect();
                                if e > 0.003 && !kids.is_empty() {
                                    let full = kids.len() as f32 * CHAT_ROW;
                                    let (vis, _) = ui.allocate_exact_size(vec2(w, full * e), Sense::hover());
                                    let clip = vis.intersect(ui.clip_rect());
                                    let pc = ui.painter().with_clip_rect(clip);
                                    for (k, &ci) in kids.iter().enumerate() {
                                        let rr = Rect::from_min_size(
                                            pos2(vis.left(), vis.top() - full * (1.0 - e) + k as f32 * CHAT_ROW),
                                            vec2(w, CHAT_ROW),
                                        );
                                        let hr = rr.intersect(clip);
                                        if hr.height() < 2.0 {
                                            continue;
                                        }
                                        let o = side_row_at(
                                            ui, &pc, rr, hr, Id::new(("chat", ci)), &self.chats[ci].title, None,
                                            ci == self.cur, 26.0, false, None,
                                        );
                                        if o.clicked {
                                            acts.push(Act::Select(ci));
                                        }
                                        if o.delete {
                                            acts.push(Act::AskDelete(Del::Chat(ci)));
                                        }
                                    }
                                }
                            }
                            for ci in (0..self.chats.len()).rev() {
                                let c = &self.chats[ci];
                                if c.project.is_none() && !c.msgs.is_empty() {
                                    let w = ui.available_width();
                                    let (row, _) = ui.allocate_exact_size(vec2(w, CHAT_ROW), Sense::hover());
                                    let p = ui.painter().clone();
                                    let o = side_row_at(
                                        ui, &p, row, row, Id::new(("chat", ci)), &c.title, None, ci == self.cur, 0.0,
                                        false, None,
                                    );
                                    if o.clicked {
                                        acts.push(Act::Select(ci));
                                    }
                                    if o.delete {
                                        acts.push(Act::AskDelete(Del::Chat(ci)));
                                    }
                                }
                            }
                        });
                });

                // ── background controls ──
                let bot = Rect::from_min_max(pos2(inner.left(), inner.bottom() - BOT_H), inner.max);
                let pp = ui.painter().clone();
                pp.line_segment(
                    [pos2(bot.left() + 4.0, bot.top()), pos2(bot.right() - 4.0, bot.top())],
                    Stroke::new(1.0, Color32::from_black_alpha(22)),
                );
                pp.text(
                    pos2(bot.left() + 6.0, bot.top() + 20.0),
                    Align2::LEFT_CENTER,
                    "Wallpaper blur",
                    FontId::proportional(13.0),
                    muted(),
                );
                let srect = Rect::from_min_size(pos2(bot.left(), bot.top() + 34.0), vec2(bot.width(), 30.0));
                pill_slider(ui, srect, Id::new("blur_slider"), &mut blur_v, 0.0, BLUR_MAX);

                let btn = Rect::from_min_size(pos2(bot.left(), bot.bottom() - 42.0), vec2(bot.width(), 40.0));
                let (r, h) = hit(ui, btn, Id::new("bg_pick"));
                pp.rect_filled(btn, Rounding::same(20.0), Color32::from_white_alpha((70.0 + h * 80.0) as u8));
                pp.rect_stroke(btn, Rounding::same(20.0), Stroke::new(1.0, Color32::from_white_alpha(100)));
                let tr = pp.text(
                    btn.center() + vec2(12.0, 0.0),
                    Align2::CENTER_CENTER,
                    "Change background",
                    FontId::proportional(13.0),
                    INK,
                );
                let ic = pos2(tr.left() - 17.0, btn.center().y);
                let st = Stroke::new(1.4, INK);
                pp.rect_stroke(Rect::from_center_size(ic, vec2(16.0, 13.0)), Rounding::same(3.0), st);
                pp.circle_filled(ic + vec2(3.5, -2.0), 1.6, INK);
                pp.add(Shape::line(
                    vec![ic + vec2(-6.0, 4.0), ic + vec2(-2.0, -0.5), ic + vec2(1.0, 2.5), ic + vec2(3.0, 0.5), ic + vec2(6.0, 4.0)],
                    Stroke::new(1.3, INK),
                ));
                if r.clicked() {
                    acts.push(Act::PickBg);
                }
            },
        );
        if (blur_v - self.blur_amt).abs() > 1e-6 {
            self.dirty = true;
        }
        self.blur_amt = blur_v;
        for a in acts {
            match a {
                Act::NewChat(pj) => self.new_chat(pj),
                Act::Select(i) => {
                    self.cur = i;
                    self.dirty = true;
                }
                Act::ToggleProject(i) => {
                    self.projects[i].open = !self.projects[i].open;
                    self.cur_project = Some(i);
                    self.dirty = true;
                }
                Act::AskDelete(d) => {
                    self.confirm_text = match d {
                        Del::Chat(ci) => (
                            "Delete this chat?".to_string(),
                            format!(
                                "“{}” will be permanently deleted. This can’t be undone.",
                                self.chats.get(ci).map(|c| trunc(&c.title, 40)).unwrap_or_default()
                            ),
                        ),
                        Del::Project(pi) => {
                            let n = self.chats.iter().filter(|c| c.project == Some(pi)).count();
                            (
                                "Remove this project?".to_string(),
                                format!(
                                    "“{}” and its {} chat{} will be removed from Kasandra. The folder and files on your disk are not touched.",
                                    self.projects.get(pi).map(|p| trunc(&p.name, 30)).unwrap_or_default(),
                                    n,
                                    if n == 1 { "" } else { "s" }
                                ),
                            )
                        }
                    };
                    self.confirm = Some(d);
                }
                Act::PickBg => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp"])
                        .pick_file()
                    {
                        if let Ok(buf) = load_bg(&path) {
                            self.set_bg(buf);
                            self.bg_path = Some(path.display().to_string());
                            self.dirty = true;
                        }
                    }
                }
                Act::NewProject => {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        let name = path
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Project".to_string());
                        self.projects.push(Project { name, path: path.display().to_string(), open: true });
                        let i = self.projects.len() - 1;
                        self.new_chat(i);
                        self.dirty = true;
                    }
                }
            }
        }
    }

    // ───────────── chat area ─────────────
    fn chat_ui(&mut self, ctx: &Context, side_t: f32, bg: &Bg) {
        let screen = ctx.screen_rect();
        let left = (SIDEBAR_W + 24.0) * side_t;
        let area = Rect::from_min_max(pos2(left, TITLE_H), screen.max);
        let label = match &self.active {
            Some((pi, m)) => format!("{} · {}", self.providers[*pi].name, m),
            None => "Select AI".to_string(),
        };
        let has_active = self.active.is_some();
        let pending = self.chats[self.cur].pending;
        let empty = self.chats[self.cur].msgs.is_empty();
        let chat_id = self.chats[self.cur].id;
        let proj_name = self.chats[self.cur].project.and_then(|i| self.projects.get(i)).map(|p| p.name.clone());
        let (mut open_popup, mut do_send) = (false, false);

        Area::new(Id::new("chat")).order(Order::Middle).fixed_pos(area.min).constrain(false).show(ctx, |ui| {
            ui.set_min_size(area.size());
            let cw = (area.width() - 48.0).min(820.0).max(200.0);
            let cx = area.center().x;
            let bar = Rect::from_center_size(pos2(cx, area.bottom() - 46.0), vec2(cw, 54.0));
            let mrect = Rect::from_min_max(
                pos2(cx - cw / 2.0, area.top() + 10.0),
                pos2(cx + cw / 2.0, bar.top() - 14.0),
            );

            if empty && !pending {
                let (head, sub) = match &proj_name {
                    Some(n) => (
                        format!("Ask anything about {n}"),
                        "Kasandra reads the files in this folder with every message".to_string(),
                    ),
                    None => ("What’s on your mind?".to_string(), "Add a project in the sidebar to give the AI access to a folder".to_string()),
                };
                let c = mrect.center();
                ui.painter().text(c - vec2(0.0, 12.0), Align2::CENTER_CENTER, head, FontId::new(22.0, bold_family()), fa(Color32::WHITE, 0.95));
                ui.painter().text(c + vec2(0.0, 16.0), Align2::CENTER_CENTER, sub, FontId::proportional(13.5), fa(Color32::WHITE, 0.8));
            } else {
                ui.allocate_ui_at_rect(mrect, |ui| {
                    egui::ScrollArea::vertical()
                        .id_source(("msgs", chat_id))
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.add_space(4.0);
                            for (mi, m) in self.chats[self.cur].msgs.iter().enumerate() {
                                bubble(ui, m, Id::new(("msg", chat_id, mi)));
                            }
                            if pending {
                                typing(ui);
                            }
                            ui.add_space(6.0);
                        });
                });
            }

            glass(ui.painter(), bg, bar, 27.0, 1.0);
            ui.allocate_ui_at_rect(bar.shrink2(vec2(10.0, 8.0)), |ui| {
                ui.horizontal_centered(|ui| {
                    if model_chip(ui, &label, has_active).clicked() {
                        open_popup = true;
                    }
                    let w = (ui.available_width() - 38.0 - ui.spacing().item_spacing.x * 2.0).max(40.0);
                    let te = ui.add_sized(
                        [w, 24.0],
                        egui::TextEdit::singleline(&mut self.input).frame(false).hint_text("Message Kasandra…"),
                    );
                    if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        do_send = true;
                        te.request_focus();
                    }
                    let (r, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
                    let (resp, h) = hit(ui, r, Id::new("send_btn"));
                    let col = if pending { fa(GREEN, 0.4) } else { lerpc(GREEN, GREEN_DK, h) };
                    ui.painter().circle_filled(r.center(), 19.0, col);
                    draw_arrow_up(ui.painter(), r.center(), 7.0, Color32::WHITE);
                    if resp.clicked() {
                        do_send = true;
                    }
                });
            });
        });
        if open_popup {
            self.popup = true;
        }
        if do_send {
            self.send(ctx);
        }
    }

    // ───────────── delete confirmation ─────────────
    fn confirm_ui(&mut self, ctx: &Context, bg: &Bg) {
        let t = ease(ctx.animate_bool_with_time(Id::new("confirm_t"), self.confirm.is_some(), 0.10));
        if t < 0.002 {
            return;
        }
        let screen = ctx.screen_rect();
        let (title, body) = self.confirm_text.clone();
        let mut decision: Option<bool> = None;
        Area::new(Id::new("confirm")).order(Order::Debug).fixed_pos(screen.min).constrain(false).show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            let p = ui.painter().clone();
            p.add(Shape::convex_polygon(
                rounded_poly(screen, bg.win_r, 10),
                Color32::from_black_alpha(90).gamma_multiply(t),
                Stroke::NONE,
            ));
            let w = 400.0_f32.min(screen.width() - 32.0);
            let rect = Rect::from_center_size(screen.center() + vec2(0.0, (1.0 - t) * 12.0), vec2(w, 196.0));
            glass(&p, bg, rect, PANEL_R + 4.0, t);
            let inner = rect.shrink2(vec2(26.0, 24.0));
            p.text(inner.left_top(), Align2::LEFT_TOP, title, FontId::new(19.0, bold_family()), fa(INK, t));
            let g = p.layout(body, FontId::proportional(13.5), fa(INK, t * 0.7), inner.width());
            p.galley(inner.left_top() + vec2(0.0, 34.0), g, fa(INK, t * 0.7));

            let bw = (inner.width() - 12.0) / 2.0;
            let y = inner.bottom() - 42.0;
            let cancel = Rect::from_min_size(pos2(inner.left(), y), vec2(bw, 42.0));
            let del = Rect::from_min_size(pos2(inner.left() + bw + 12.0, y), vec2(bw, 42.0));

            let (r, h) = hit(ui, cancel, Id::new("cf_cancel"));
            p.rect_filled(cancel, Rounding::same(21.0), Color32::from_white_alpha((90.0 + h * 90.0) as u8).gamma_multiply(t));
            p.text(cancel.center(), Align2::CENTER_CENTER, "Cancel", FontId::proportional(14.0), fa(INK, t));
            if r.clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                decision = Some(false);
            }

            let (r, h) = hit(ui, del, Id::new("cf_delete"));
            p.rect_filled(del, Rounding::same(21.0), fa(lerpc(RED, RED_DK, h), t));
            p.text(del.center(), Align2::CENTER_CENTER, "Delete", FontId::new(14.0, bold_family()), fa(Color32::WHITE, t));
            if r.clicked() {
                decision = Some(true);
            }
        });
        match decision {
            Some(true) => {
                if let Some(d) = self.confirm.take() {
                    match d {
                        Del::Chat(ci) => self.delete_chat(ci),
                        Del::Project(pi) => self.delete_project(pi),
                    }
                }
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    // ───────────── API popup ─────────────
    fn popup_ui(&mut self, ctx: &Context, bg: &Bg) {
        let t = ease(ctx.animate_bool_with_time(Id::new("popup_t"), self.popup, 0.10));
        if t < 0.002 {
            return;
        }
        let screen = ctx.screen_rect();
        let now = ctx.input(|i| i.time);
        let active = self.active.clone();
        let (mut close, mut add) = (false, false);
        let mut remove: Option<usize> = None;
        let mut choose: Option<(usize, String)> = None;

        Area::new(Id::new("popup")).order(Order::Debug).fixed_pos(screen.min).constrain(false).show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            let p = ui.painter().clone();
            p.add(Shape::convex_polygon(
                rounded_poly(screen, bg.win_r, 10),
                Color32::from_black_alpha(80).gamma_multiply(t),
                Stroke::NONE,
            ));

            let pw = (screen.width() - 32.0).min(640.0);
            let ph = (screen.height() - 32.0).min(370.0);
            let rect = Rect::from_center_size(screen.center() + vec2(0.0, (1.0 - t) * 14.0), vec2(pw, ph));
            glass(&p, bg, rect, PANEL_R + 4.0, t);

            let inner = rect.shrink(18.0);
            let top = inner.top();
            let xr = Rect::from_center_size(pos2(inner.right() - 15.0, top + 20.0), vec2(30.0, 30.0));
            let chk = Rect::from_center_size(pos2(xr.left() - 30.0, top + 20.0), vec2(40.0, 40.0));
            let field = Rect::from_min_max(pos2(inner.left(), top), pos2(chk.left() - 10.0, top + 40.0));

            p.rect_filled(field, Rounding::same(20.0), Color32::from_white_alpha(90).gamma_multiply(t));
            let te_rect = Rect::from_center_size(field.center(), vec2(field.width() - 28.0, 22.0));
            let te = ui.put(
                te_rect,
                egui::TextEdit::singleline(&mut self.key_input)
                    .frame(false)
                    .password(true)
                    .hint_text("Paste an API key   (custom: https://host/v1 key)")
                    .text_color(fa(INK, t)),
            );
            if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                add = true;
            }

            let (r, h) = hit(ui, chk, Id::new("pop_chk"));
            p.circle_filled(chk.center(), 20.0, fa(lerpc(GREEN, GREEN_DK, h), t));
            if self.adding {
                ui.put(
                    Rect::from_center_size(chk.center(), vec2(22.0, 22.0)),
                    egui::Spinner::new().color(Color32::WHITE),
                );
            } else {
                draw_check(&p, chk.center(), 8.0, fa(Color32::WHITE, t));
                if r.clicked() {
                    add = true;
                }
            }

            let (r, h) = hit(ui, xr, Id::new("pop_x"));
            p.circle_filled(xr.center(), 15.0, Color32::from_white_alpha((70.0 + h * 90.0) as u8).gamma_multiply(t));
            draw_x(&p, xr.center(), 4.5, fa(INK, t));
            if r.clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }

            if let Some(e) = &self.add_err {
                p.text(
                    pos2(inner.left() + 8.0, field.bottom() + 14.0),
                    Align2::LEFT_CENTER,
                    trunc(e, 95),
                    FontId::proportional(12.0),
                    fa(Color32::from_rgb(190, 40, 60), t),
                );
            }

            let cr = Rect::from_min_max(pos2(inner.left(), field.bottom() + 28.0), inner.max);
            if self.providers.is_empty() {
                p.text(
                    cr.center(),
                    Align2::CENTER_CENTER,
                    "Paste a key above and press the green check.\nProviders and their models show up here as cards.\nKeys are stored in your system's secure credential store.",
                    FontId::proportional(13.5),
                    fa(muted(), t),
                );
            } else {
                let ch = (cr.height() - 18.0).max(120.0);
                ui.allocate_ui_at_rect(cr, |ui| {
                    egui::ScrollArea::horizontal().id_source("cards").auto_shrink([false, false]).show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            for (pi, prov) in self.providers.iter().enumerate() {
                                let born = ((now - prov.born) / 0.25).clamp(0.0, 1.0) as f32;
                                if born < 1.0 {
                                    ui.ctx().request_repaint();
                                }
                                let ca = ease(born) * t;
                                let (slot, _) = ui.allocate_exact_size(vec2(184.0, ch), Sense::hover());
                                let card = slot.translate(vec2(0.0, (1.0 - ease(born)) * 14.0));
                                let is_act = active.as_ref().map_or(false, |(a, _)| *a == pi);
                                p.rect_filled(card, Rounding::same(16.0), Color32::from_white_alpha(70).gamma_multiply(ca));
                                if is_act {
                                    p.rect_stroke(card, Rounding::same(16.0), Stroke::new(1.5, fa(GREEN, ca)));
                                }
                                p.text(
                                    pos2(card.left() + 14.0, card.top() + 20.0),
                                    Align2::LEFT_CENTER,
                                    trunc(&prov.name, 15),
                                    FontId::proportional(17.0),
                                    fa(INK, ca),
                                );
                                p.text(
                                    pos2(card.left() + 14.0, card.top() + 40.0),
                                    Align2::LEFT_CENTER,
                                    format!("{} models", prov.models.len()),
                                    FontId::proportional(11.5),
                                    fa(muted(), ca),
                                );
                                let rr = Rect::from_center_size(pos2(card.right() - 18.0, card.top() + 20.0), vec2(22.0, 22.0));
                                let (r, h) = hit(ui, rr, Id::new(("rm", pi)));
                                p.circle_filled(rr.center(), 11.0, Color32::from_white_alpha((h * 120.0) as u8).gamma_multiply(ca));
                                draw_x(&p, rr.center(), 3.5, fa(muted(), ca));
                                if r.clicked() {
                                    remove = Some(pi);
                                }
                                let lr = Rect::from_min_max(
                                    pos2(card.left() + 6.0, card.top() + 56.0),
                                    pos2(card.right() - 6.0, card.bottom() - 8.0),
                                );
                                ui.allocate_ui_at_rect(lr, |ui| {
                                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                        egui::ScrollArea::vertical()
                                            .id_source(("ml", pi))
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                for m in &prov.models {
                                                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                                                    let sel = active.as_ref().map_or(false, |(a, b)| *a == pi && b == m);
                                                    let (r, h) = hit(ui, row, Id::new(("m", pi, m.as_str())));
                                                    let fill = if sel {
                                                        fa(GREEN, ca)
                                                    } else {
                                                        Color32::from_white_alpha((h * 110.0) as u8).gamma_multiply(ca)
                                                    };
                                                    ui.painter().rect_filled(row, Rounding::same(8.0), fill);
                                                    ui.painter().text(
                                                        row.left_center() + vec2(10.0, 0.0),
                                                        Align2::LEFT_CENTER,
                                                        trunc(m, 22),
                                                        FontId::proportional(12.5),
                                                        fa(if sel { Color32::WHITE } else { INK }, ca),
                                                    );
                                                    if r.clicked() {
                                                        choose = Some((pi, m.clone()));
                                                    }
                                                    r.on_hover_text(m.as_str());
                                                }
                                            });
                                    });
                                });
                            }
                        });
                    });
                });
            }
        });

        if close {
            self.popup = false;
        }
        if add {
            self.start_add(ctx);
        }
        if let Some(i) = remove {
            key_delete(&self.providers[i]);
            self.providers.remove(i);
            if let Some((a, m)) = self.active.take() {
                if a > i {
                    self.active = Some((a - 1, m));
                } else if a < i {
                    self.active = Some((a, m));
                }
            }
            self.dirty = true;
        }
        if let Some(c) = choose {
            self.active = Some(c);
            self.dirty = true;
        }
    }
}

// ───────────────────────────── widgets ─────────────────────────────

struct RowOut {
    clicked: bool,
    plus: bool,
    delete: bool,
}

fn section_header(ui: &mut Ui, label: &str, id: Id) -> bool {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        muted(),
    );
    let pr = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(24.0, 24.0));
    let (r, h) = hit(ui, pr, id);
    ui.painter().circle_filled(pr.center(), 12.0, Color32::from_white_alpha((40.0 + h * 130.0) as u8));
    draw_plus(ui.painter(), pr.center(), 5.0, INK);
    r.clicked()
}

/// A sidebar row drawn at an explicit rect (so children can slide/clip while expanding).
/// `chev` = Some(expand 0..1) draws a rotating chevron (project rows).
/// Right-clicking the row opens a Delete menu.
fn side_row_at(
    ui: &mut Ui,
    p: &Painter,
    rect: Rect,
    hit_rect: Rect,
    id: Id,
    title: &str,
    sub: Option<&str>,
    selected: bool,
    indent: f32,
    plus: bool,
    chev: Option<f32>,
) -> RowOut {
    let (resp, h) = hit(ui, hit_rect, id);
    let mut out = RowOut { clicked: false, plus: false, delete: false };
    let body = rect.shrink2(vec2(0.0, 1.0));
    let fill = if selected {
        Color32::from_black_alpha(34)
    } else {
        Color32::from_white_alpha((h * 110.0) as u8)
    };
    p.rect_filled(body, Rounding::same(10.0), fill);
    if selected && chev.is_none() {
        p.rect_filled(
            Rect::from_min_size(pos2(body.left() + 3.0 + indent - 10.0, body.top() + 10.0), vec2(3.0, body.height() - 20.0)),
            Rounding::same(2.0),
            Color32::from_gray(110),
        );
    }
    let mut tx = rect.left() + 14.0 + indent;
    if let Some(e) = chev {
        let c = pos2(rect.left() + 17.0, rect.center().y);
        let a = e * std::f32::consts::FRAC_PI_2;
        let rot = |x: f32, y: f32| c + vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos());
        let s = Stroke::new(1.8, INK);
        p.line_segment([rot(-2.0, -4.0), rot(2.0, 0.0)], s);
        p.line_segment([rot(2.0, 0.0), rot(-2.0, 4.0)], s);
        tx += 18.0;
    }
    let ty = rect.center().y - if sub.is_some() { 7.0 } else { 0.0 };
    p.text(pos2(tx, ty), Align2::LEFT_CENTER, trunc(title, 22), FontId::proportional(14.5), INK);
    if let Some(s) = sub {
        p.text(pos2(tx, ty + 16.0), Align2::LEFT_CENTER, trunc(s, 30), FontId::proportional(10.5), muted());
    }
    if plus {
        let pr = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(24.0, 24.0));
        let (r, ph) = hit(ui, pr, id.with("plus"));
        p.circle_filled(pr.center(), 12.0, Color32::from_white_alpha((ph * 170.0) as u8));
        draw_plus(p, pr.center(), 4.5, INK);
        out.plus = r.clicked();
    }
    out.clicked = resp.clicked() && !out.plus;

    // right-click menu
    let menu_label = if chev.is_some() { "Remove project" } else { "Delete chat" };
    resp.context_menu(|ui| {
        ui.set_min_width(150.0);
        if ui.button(RichText::new(menu_label).color(RED)).clicked() {
            out.delete = true;
            ui.close_menu();
        }
    });
    out
}

fn model_chip(ui: &mut Ui, label: &str, on: bool) -> Response {
    let g = ui.painter().layout_no_wrap(trunc(label, 28), FontId::proportional(13.0), INK);
    let (rect, _) = ui.allocate_exact_size(vec2(g.size().x + 42.0, 34.0), Sense::hover());
    let (resp, h) = hit(ui, rect, Id::new("model_chip"));
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(17.0), Color32::from_white_alpha((70.0 + h * 90.0) as u8));
    p.circle_filled(
        pos2(rect.left() + 16.0, rect.center().y),
        4.5,
        if on { GREEN } else { Color32::from_gray(150) },
    );
    let pos = pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0);
    p.galley(pos, g, INK);
    resp
}

fn bubble(ui: &mut Ui, m: &Msg, salt: Id) {
    let maxw = (ui.available_width() * 0.84 - 32.0).max(120.0);
    let layout = if m.user { Layout::right_to_left(Align::TOP) } else { Layout::left_to_right(Align::TOP) };
    ui.with_layout(layout, |ui| {
        ui.vertical(|ui| {
            let (fill, col) = if m.user {
                (GREEN, Color32::WHITE)
            } else if m.err {
                (Color32::from_rgb(255, 229, 231), Color32::from_rgb(150, 30, 45))
            } else {
                (Color32::from_white_alpha(185), INK)
            };
            egui::Frame::none()
                .fill(fill)
                .rounding(Rounding::same(18.0))
                .inner_margin(Margin::symmetric(16.0, 12.0))
                .show(ui, |ui| {
                    ui.set_max_width(maxw);
                    if m.user || m.err {
                        ui.add(
                            egui::Label::new(RichText::new(&m.text).size(BODY_SIZE).line_height(Some(LINE_H)).color(col))
                                .wrap_mode(egui::TextWrapMode::Wrap),
                        );
                    } else {
                        // long / structured replies get the full bubble width so code and tables have room
                        let wide = m.text.len() > 280 || m.text.contains("```") || m.text.contains("\n|");
                        if wide {
                            ui.set_min_width(maxw);
                        }
                        md_ui(ui, &m.text, col, salt);
                    }
                });
            if !m.user && !m.err {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    if !m.meta.is_empty() {
                        ui.label(RichText::new(&m.meta).size(11.0).color(fa(Color32::WHITE, 0.75)));
                    }
                    let r = ui.add(
                        egui::Label::new(RichText::new("Copy").size(11.0).color(fa(Color32::WHITE, 0.9)))
                            .sense(Sense::click()),
                    );
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                    if r.clicked() {
                        ui.output_mut(|o| o.copied_text = m.text.clone());
                    }
                });
            }
        });
    });
    ui.add_space(14.0);
}

fn typing(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(64.0, 36.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(18.0), Color32::from_white_alpha(170));
    let t = ui.input(|i| i.time) as f32;
    for k in 0..3 {
        let kf = k as f32;
        let ph = (t * 4.0 - kf * 0.6).sin() * 0.5 + 0.5;
        p.circle_filled(pos2(rect.left() + 20.0 + kf * 12.0, rect.center().y - ph * 3.0), 3.0 + ph, GREEN);
    }
    ui.ctx().request_repaint();
}

// ───────────────────────────── eframe glue ─────────────────────────────

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if blur_r(self.blur_amt) != self.blur_applied {
            self.apply_blur();
        }

        let screen = ctx.screen_rect();
        let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let ia = 16.0 / 9.0;
        let a = screen.width() / screen.height().max(1.0);
        let uv = if a > ia {
            let hh = ia / a;
            Rect::from_min_max(pos2(0.0, (1.0 - hh) / 2.0), pos2(1.0, (1.0 + hh) / 2.0))
        } else {
            let ww = a / ia;
            Rect::from_min_max(pos2((1.0 - ww) / 2.0, 0.0), pos2((1.0 + ww) / 2.0, 1.0))
        };
        let bg = Bg {
            sharp: self.bg.id(),
            blur: self.bg_blur.id(),
            grain: self.grain.id(),
            screen,
            uv,
            win_r: if maxed { 0.0 } else { WIN_R },
        };
        let painter = ctx.layer_painter(egui::LayerId::background());
        tex_poly(&painter, &bg, bg.sharp, &rounded_poly(screen, bg.win_r, 12), Color32::WHITE);

        let side_t = ease(ctx.animate_bool_with_time(Id::new("side_t"), self.sidebar, 0.17));
        self.chat_ui(ctx, side_t, &bg);
        self.sidebar_ui(ctx, side_t, &bg);
        self.titlebar(ctx);
        self.burger(ctx, side_t);
        self.popup_ui(ctx, &bg);
        self.confirm_ui(ctx, &bg);
        self.flush(ctx);
    }
}

fn main() -> eframe::Result<()> {
    let mut vb = egui::ViewportBuilder::default()
        .with_title("Kasandra")
        .with_app_id("kasandra")
        .with_inner_size([1120.0, 680.0])
        .with_min_inner_size([640.0, 420.0])
        .with_decorations(false)
        .with_transparent(true);
    if let Some(im) = icon_rgba(256) {
        let (w, h) = (im.width(), im.height());
        vb = vb.with_icon(egui::IconData { rgba: im.into_raw(), width: w, height: h });
    }
    let opts = eframe::NativeOptions { viewport: vb, ..Default::default() };
    eframe::run_native("Kasandra", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
