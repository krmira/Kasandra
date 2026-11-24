// Kasandra - a tiny multi-provider chat client. One file, egui.
// Providers: OpenAI-compatible (OpenAI, Groq, OpenRouter, xAI, Cerebras, custom), Anthropic, Gemini.

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Area, Color32, Context, CursorIcon, FontId, Id, Key, Layout,
    Margin, Order, Painter, Pos2, Rect, Response, RichText, Rounding, Sense, Shape, Stroke,
    TextureHandle, Ui,
};
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Receiver, Sender};

const TITLE_H: f32 = 34.0;
const SIDEBAR_W: f32 = 290.0;
const WIN_R: f32 = 18.0; // window corner radius
const PANEL_R: f32 = 20.0; // glass panel radius
const GLASS_A: f32 = 115.0; // 45% white
const GREEN: Color32 = Color32::from_rgb(46, 204, 98);
const GREEN_DK: Color32 = Color32::from_rgb(30, 168, 78);
const INK: Color32 = Color32::from_rgb(20, 26, 42);

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

/// Clickable rect with an animated hover value (0..1).
fn hit(ui: &mut Ui, rect: Rect, id: Id) -> (Response, f32) {
    let r = ui.interact(rect, id, Sense::click());
    let h = ui.ctx().animate_bool_with_time(id.with("hover"), r.hovered(), 0.12);
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

/// Convex polygon filled with the background texture at the matching screen position.
fn tex_poly(p: &Painter, bg: &Bg, tex: egui::TextureId, pts: &[Pos2], tint: Color32) {
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
    let uv = |q: Pos2| {
        pos2(
            bg.uv.min.x + (q.x - bg.screen.min.x) / bg.screen.width() * bg.uv.width(),
            bg.uv.min.y + (q.y - bg.screen.min.y) / bg.screen.height() * bg.uv.height(),
        )
    };
    let mut m = egui::Mesh::with_texture(tex);
    m.vertices.push(egui::epaint::Vertex { pos: cen, uv: uv(cen), color: tint });
    for q in pts {
        m.vertices.push(egui::epaint::Vertex { pos: *q, uv: uv(*q), color: tint });
    }
    for i in 0..n {
        m.indices.extend_from_slice(&[0, (1 + i) as u32, (1 + (i + 1) % n) as u32]);
    }
    p.add(Shape::mesh(m));
}

/// Real frosted glass: blurred copy of the background + 45% white tint, one uniform layer.
fn glass(p: &Painter, bg: &Bg, rect: Rect, rad: f32, k: f32) {
    let pts = rounded_poly(rect, rad, 10);
    tex_poly(p, bg, bg.blur, &pts, Color32::WHITE.gamma_multiply(k));
    p.add(Shape::convex_polygon(
        pts,
        Color32::from_white_alpha((GLASS_A * k) as u8),
        Stroke::new(1.0, Color32::from_white_alpha((90.0 * k) as u8)),
    ));
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
fn make_bg() -> (egui::ColorImage, egui::ColorImage) {
    const W: usize = 960;
    const H: usize = 540;
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

            // reeded stripes inside a bulging arc
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
    let mut sharp = buf.clone();
    box_blur(&mut sharp, W, H, 2);
    // heavily blurred, low-res copy for frosted glass
    let (sw, sh) = (W / 4, H / 4);
    let mut small = vec![[0f32; 3]; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            let mut s = [0f32; 3];
            for dy in 0..4 {
                for dx in 0..4 {
                    let p = buf[(y * 4 + dy) * W + x * 4 + dx];
                    s[0] += p[0];
                    s[1] += p[1];
                    s[2] += p[2];
                }
            }
            small[y * sw + x] = [s[0] / 16.0, s[1] / 16.0, s[2] / 16.0];
        }
    }
    box_blur(&mut small, sw, sh, 5);
    box_blur(&mut small, sw, sh, 5);
    (to_img(&sharp, W, H), to_img(&small, sw, sh))
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

// ───────────────────────────── providers / network ─────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    OpenAi,
    Anthropic,
    Gemini,
}

#[derive(Clone)]
struct Provider {
    kind: Kind,
    name: String,
    base: String,
    key: String,
    models: Vec<String>,
    born: f64,
}

fn detect(input: &str) -> Result<Provider, String> {
    let input = input.trim();
    if input.is_empty() {
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
    // custom OpenAI-compatible endpoint: "<base url> <key>"
    if let Some((b, k)) = input.split_once(char::is_whitespace) {
        if b.starts_with("http") {
            let host = b.split("//").nth(1).unwrap_or(b).split('/').next().unwrap_or(b);
            return Ok(mk(Kind::OpenAi, host, b, k.trim()));
        }
        return Err("Expected just a key, or: https://host/v1 <key>".into());
    }
    let k = input;
    Ok(if k.starts_with("sk-ant-") {
        mk(Kind::Anthropic, "Anthropic", "https://api.anthropic.com/v1", k)
    } else if k.starts_with("sk-or-") {
        mk(Kind::OpenAi, "OpenRouter", "https://openrouter.ai/api/v1", k)
    } else if k.starts_with("gsk_") {
        mk(Kind::OpenAi, "Groq", "https://api.groq.com/openai/v1", k)
    } else if k.starts_with("xai-") {
        mk(Kind::OpenAi, "xAI", "https://api.x.ai/v1", k)
    } else if k.starts_with("csk-") {
        mk(Kind::OpenAi, "Cerebras", "https://api.cerebras.ai/v1", k)
    } else if k.starts_with("AIza") {
        mk(Kind::Gemini, "Google Gemini", "https://generativelanguage.googleapis.com/v1beta", k)
    } else if k.starts_with("sk-") {
        mk(Kind::OpenAi, "OpenAI", "https://api.openai.com/v1", k)
    } else {
        return Err("Unknown key format. For other providers paste: https://host/v1 <key>".into());
    })
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(120)).build()
}
fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(c, r) => {
            let body = r.into_string().unwrap_or_default();
            format!("HTTP {c}: {}", body.chars().take(160).collect::<String>())
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
    let a = agent();
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
    out.sort();
    out.dedup();
    if out.is_empty() {
        Err("No models returned for this key".into())
    } else {
        Ok(out)
    }
}

fn chat_call(p: &Provider, model: &str, h: &[(bool, String)]) -> Result<String, String> {
    let a = agent();
    let empty = || "Empty response".to_string();
    match p.kind {
        Kind::OpenAi => {
            let msgs: Vec<Value> = h
                .iter()
                .map(|(u, t)| json!({"role": if *u {"user"} else {"assistant"}, "content": t}))
                .collect();
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
                json!({"model": model, "max_tokens": 4096, "messages": msgs}),
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
                json!({ "contents": contents }),
            )?;
            v["candidates"][0]["content"]["parts"][0]["text"].as_str().map(str::to_string).ok_or_else(empty)
        }
    }
}

// ───────────────────────────── app state ─────────────────────────────

struct Msg {
    user: bool,
    text: String,
}
struct Chat {
    title: String,
    project: Option<usize>,
    msgs: Vec<Msg>,
    pending: bool,
}
struct Project {
    name: String,
    path: String,
    open: bool,
}
enum Ev {
    Added(Result<Provider, String>),
    Reply(usize, Result<String, String>),
}
enum Act {
    NewChat(usize),
    Select(usize),
    NewProject,
    ToggleProject(usize),
}

struct App {
    bg: TextureHandle,
    bg_blur: TextureHandle,
    sidebar: bool,
    popup: bool,
    chats: Vec<Chat>,
    cur: usize,
    projects: Vec<Project>,
    cur_project: Option<usize>,
    providers: Vec<Provider>,
    active: Option<(usize, String)>,
    key_input: String,
    adding: bool,
    add_err: Option<String>,
    input: String,
    tx: Sender<Ev>,
    rx: Receiver<Ev>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
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
        let (sharp, blur) = make_bg();
        let bg = cc.egui_ctx.load_texture("bg", sharp, egui::TextureOptions::LINEAR);
        let bg_blur = cc.egui_ctx.load_texture("bg_blur", blur, egui::TextureOptions::LINEAR);
        let (tx, rx) = channel();
        Self {
            bg,
            bg_blur,
            sidebar: false,
            popup: false,
            chats: vec![Chat { title: "New chat".into(), project: None, msgs: vec![], pending: false }],
            cur: 0,
            projects: vec![],
            cur_project: None,
            providers: vec![],
            active: None,
            key_input: String::new(),
            adding: false,
            add_err: None,
            input: String::new(),
            tx,
            rx,
        }
    }

    fn poll(&mut self, ctx: &Context) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Ev::Added(Ok(mut p)) => {
                    self.adding = false;
                    self.key_input.clear();
                    p.born = ctx.input(|i| i.time);
                    let first = p.models.first().cloned();
                    let idx = if let Some(i) = self.providers.iter().position(|q| q.key == p.key) {
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
                }
                Ev::Added(Err(e)) => {
                    self.adding = false;
                    self.add_err = Some(e);
                }
                Ev::Reply(ci, r) => {
                    if let Some(c) = self.chats.get_mut(ci) {
                        c.pending = false;
                        let text = match r {
                            Ok(t) => t,
                            Err(e) => format!("Error: {e}"),
                        };
                        c.msgs.push(Msg { user: false, text });
                    }
                }
            }
        }
    }

    /// New chat inside project `pj` (re-uses an empty one if it exists).
    fn new_chat(&mut self, pj: usize) {
        self.projects[pj].open = true;
        self.cur_project = Some(pj);
        if let Some(i) = self.chats.iter().position(|c| c.msgs.is_empty() && c.project == Some(pj)) {
            self.cur = i;
            return;
        }
        self.chats.push(Chat { title: "New chat".into(), project: Some(pj), msgs: vec![], pending: false });
        self.cur = self.chats.len() - 1;
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
        let cur_project = self.cur_project;
        let chat = &mut self.chats[self.cur];
        if chat.msgs.is_empty() {
            chat.title = trunc(&text, 28);
            if chat.project.is_none() {
                chat.project = cur_project; // loose chat gets filed under the selected project
            }
        }
        chat.msgs.push(Msg { user: true, text });
        chat.pending = true;
        let hist: Vec<(bool, String)> = chat.msgs.iter().map(|m| (m.user, m.text.clone())).collect();
        self.input.clear();
        let (tx, idx, ctx) = (self.tx.clone(), self.cur, ctx.clone());
        std::thread::spawn(move || {
            let r = chat_call(&p, &model, &hist);
            let _ = tx.send(Ev::Reply(idx, r));
            ctx.request_repaint();
        });
    }

    fn start_add(&mut self, ctx: &Context) {
        if self.adding {
            return;
        }
        match detect(&self.key_input) {
            Err(e) => self.add_err = Some(e),
            Ok(p) => {
                self.adding = true;
                self.add_err = None;
                let (tx, ctx) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let r = fetch_models(&p).map(|m| Provider { models: m, ..p.clone() });
                    let _ = tx.send(Ev::Added(r));
                    ctx.request_repaint();
                });
            }
        }
    }

    // ───────────── title bar (transparent, pixel title, colored orbs) ─────────────
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
                pixel_text(&p, pos2(rect.left() + 18.0, ty + 1.0), "Kasandra", px, Color32::from_black_alpha(70));
                pixel_text(&p, pos2(rect.left() + 18.0, ty), "Kasandra", px, Color32::from_rgb(244, 247, 255));

                // orbs: minimize (ice violet), maximize (green), close (rose)
                let cy = rect.center().y;
                let xs = [rect.right() - 68.0, rect.right() - 44.0, rect.right() - 20.0];
                let cols = [Color32::from_rgb(150, 158, 235), GREEN, Color32::from_rgb(240, 98, 126)];
                let group = Rect::from_min_max(pos2(xs[0] - 12.0, cy - 12.0), pos2(xs[2] + 12.0, cy + 12.0));
                let gh = ctx.animate_bool_with_time(
                    Id::new("orbs_h"),
                    ctx.pointer_hover_pos().map_or(false, |q| group.contains(q)),
                    0.12,
                );
                for (i, cx) in xs.iter().enumerate() {
                    let c = pos2(*cx, cy);
                    let (r, h) = hit(ui, Rect::from_center_size(c, vec2(20.0, 20.0)), Id::new(("orb", i)));
                    let rad = 7.0 + h * 0.8;
                    let col = cols[i];
                    p.circle_filled(c, rad, col);
                    p.circle_stroke(c, rad, Stroke::new(1.0, lerpc(col, Color32::WHITE, 0.0).gamma_multiply(0.6)));
                    p.circle_filled(c + vec2(-1.8, -2.4), 2.6, Color32::from_white_alpha(80));
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
            let col = lerpc(Color32::WHITE, INK, t); // white on the bg, dark once on the glass
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
        Area::new(Id::new("sidebar")).order(Order::Foreground).fixed_pos(rect.min).constrain(false).show(
            ctx,
            |ui| {
                ui.set_min_size(rect.size());
                glass(ui.painter(), bg, rect, PANEL_R, 1.0);
                let inner = rect.shrink2(vec2(14.0, 12.0));
                ui.allocate_ui_at_rect(inner, |ui| {
                    ui.add_space(46.0); // room for the burger button
                    if section_header(ui, "Projects", Id::new("sh_proj")) {
                        acts.push(Act::NewProject);
                    }
                    egui::ScrollArea::vertical()
                        .max_height(inner.height() - 46.0 - 28.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.projects.is_empty() {
                                ui.label(RichText::new("  Press + to add a project folder").size(12.0).color(muted()));
                            }
                            for (pi, pr) in self.projects.iter().enumerate() {
                                let e = ease(ui.ctx().animate_bool_with_time(Id::new(("pe", pi)), pr.open, 0.22));
                                let w = ui.available_width();
                                let (row, _) = ui.allocate_exact_size(vec2(w, 38.0), Sense::hover());
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
                                let kids: Vec<usize> =
                                    (0..self.chats.len()).rev().filter(|&ci| self.chats[ci].project == Some(pi)).collect();
                                if e > 0.003 && !kids.is_empty() {
                                    let full = kids.len() as f32 * 36.0;
                                    let (vis, _) = ui.allocate_exact_size(vec2(w, full * e), Sense::hover());
                                    let clip = vis.intersect(ui.clip_rect());
                                    let pc = ui.painter().with_clip_rect(clip);
                                    for (k, &ci) in kids.iter().enumerate() {
                                        let rr = Rect::from_min_size(
                                            pos2(vis.left(), vis.top() - full * (1.0 - e) + k as f32 * 36.0),
                                            vec2(w, 36.0),
                                        );
                                        let hr = rr.intersect(clip);
                                        if hr.height() < 2.0 {
                                            continue;
                                        }
                                        let o = side_row_at(
                                            ui, &pc, rr, hr, Id::new(("chat", ci)), &self.chats[ci].title, None,
                                            ci == self.cur, 24.0, false, None,
                                        );
                                        if o.clicked {
                                            acts.push(Act::Select(ci));
                                        }
                                    }
                                }
                            }
                            // loose chats (not in a project yet) show up only once they have messages
                            for ci in (0..self.chats.len()).rev() {
                                let c = &self.chats[ci];
                                if c.project.is_none() && !c.msgs.is_empty() {
                                    let w = ui.available_width();
                                    let (row, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
                                    let p = ui.painter().clone();
                                    let o = side_row_at(
                                        ui, &p, row, row, Id::new(("chat", ci)), &c.title, None, ci == self.cur, 0.0,
                                        false, None,
                                    );
                                    if o.clicked {
                                        acts.push(Act::Select(ci));
                                    }
                                }
                            }
                        });
                });
            },
        );
        for a in acts {
            match a {
                Act::NewChat(pj) => self.new_chat(pj),
                Act::Select(i) => self.cur = i,
                Act::ToggleProject(i) => {
                    self.projects[i].open = !self.projects[i].open;
                    self.cur_project = Some(i);
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

            if !(empty && !pending) {
                ui.allocate_ui_at_rect(mrect, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                        ui.add_space(4.0);
                        for m in &self.chats[self.cur].msgs {
                            bubble(ui, m);
                        }
                        if pending {
                            typing(ui);
                        }
                        ui.add_space(6.0);
                    });
                });
            }

            // one clean pill, nothing behind it
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

    // ───────────── API popup ─────────────
    fn popup_ui(&mut self, ctx: &Context, bg: &Bg) {
        let t = ease(ctx.animate_bool_with_time(Id::new("popup_t"), self.popup, 0.16));
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
            // full-screen blocker so nothing behind is clickable
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
                    "Paste a key above and press the green check.\nProviders and their models show up here as cards.",
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
                                let born = ((now - prov.born) / 0.4).clamp(0.0, 1.0) as f32;
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
            self.providers.remove(i);
            if let Some((a, m)) = self.active.take() {
                if a > i {
                    self.active = Some((a - 1, m));
                } else if a < i {
                    self.active = Some((a, m));
                }
            }
        }
        if let Some(c) = choose {
            self.active = Some(c);
        }
    }
}

// ───────────────────────────── widgets ─────────────────────────────

struct RowOut {
    clicked: bool,
    plus: bool,
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
    let body = rect.shrink2(vec2(0.0, 1.0));
    let fill = if selected {
        GREEN.gamma_multiply(0.24)
    } else {
        Color32::from_white_alpha((h * 110.0) as u8)
    };
    p.rect_filled(body, Rounding::same(10.0), fill);
    if selected && chev.is_none() {
        p.rect_filled(
            Rect::from_min_size(pos2(body.left() + 3.0 + indent - 10.0, body.top() + 9.0), vec2(3.0, body.height() - 18.0)),
            Rounding::same(2.0),
            GREEN,
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
        tx += 16.0;
    }
    let ty = rect.center().y - if sub.is_some() { 6.0 } else { 0.0 };
    p.text(pos2(tx, ty), Align2::LEFT_CENTER, trunc(title, 22), FontId::proportional(14.0), INK);
    if let Some(s) = sub {
        p.text(pos2(tx, ty + 14.0), Align2::LEFT_CENTER, trunc(s, 30), FontId::proportional(10.5), muted());
    }
    let mut out = RowOut { clicked: false, plus: false };
    if plus {
        let pr = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(24.0, 24.0));
        let (r, ph) = hit(ui, pr, id.with("plus"));
        p.circle_filled(pr.center(), 12.0, Color32::from_white_alpha((ph * 170.0) as u8));
        draw_plus(p, pr.center(), 4.5, INK);
        out.plus = r.clicked();
    }
    out.clicked = resp.clicked() && !out.plus;
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

fn bubble(ui: &mut Ui, m: &Msg) {
    let maxw = ui.available_width() * 0.8;
    let layout = if m.user { Layout::right_to_left(Align::TOP) } else { Layout::left_to_right(Align::TOP) };
    ui.with_layout(layout, |ui| {
        let (fill, col) = if m.user {
            (GREEN, Color32::WHITE)
        } else {
            (Color32::from_white_alpha(170), INK)
        };
        egui::Frame::none()
            .fill(fill)
            .rounding(Rounding::same(16.0))
            .inner_margin(Margin::symmetric(14.0, 10.0))
            .show(ui, |ui| {
                ui.set_max_width(maxw);
                ui.add(egui::Label::new(RichText::new(&m.text).size(15.0).color(col)));
            });
    });
    ui.add_space(8.0);
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
    // fully transparent window; the rounded background is painted by us
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);

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
            screen,
            uv,
            win_r: if maxed { 0.0 } else { WIN_R },
        };
        // rounded window background
        let painter = ctx.layer_painter(egui::LayerId::background());
        tex_poly(&painter, &bg, bg.sharp, &rounded_poly(screen, bg.win_r, 12), Color32::WHITE);

        let side_t = ease(ctx.animate_bool_with_time(Id::new("side_t"), self.sidebar, 0.30));
        self.chat_ui(ctx, side_t, &bg);
        self.sidebar_ui(ctx, side_t, &bg);
        self.titlebar(ctx);
        self.burger(ctx, side_t);
        self.popup_ui(ctx, &bg);
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kasandra")
            .with_inner_size([1120.0, 680.0])
            .with_min_inner_size([640.0, 420.0])
            .with_decorations(false)
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native("Kasandra", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
