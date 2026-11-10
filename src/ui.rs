//! Immediate-mode drawing of the pill and the expanded panel. Every frame the
//! visible controls re-register themselves in `hits`, which `App` uses for
//! hover and click handling. All coordinates are logical (DPI-independent).

use crate::app::*;
use crate::gfx::*;
use crate::store::{clip_kind_of, preview, ClipKind};
use crate::sys;

const BLACK: Color = Color(0.0, 0.0, 0.0, 1.0);
const T1: Color = Color::white(0.96);
const T2: Color = Color::white(0.62);
const T3: Color = Color::white(0.38);
const SURF: Color = Color::white(0.09);
const HOVER: Color = Color::white(0.17);

/// How far the page area sits below the top of the open panel (the tab row is above it).
pub const CONTENT_Y: f32 = 40.0;

pub fn draw(a: &App, w: f32, h: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let t = a.t_expand();
    let rad = a.col_rad() + (36.0 - a.col_rad()) * t;

    // Body. It starts above the window so the top corners are clipped away and
    // the shape hangs flush from the top edge of the screen like a notch.
    // Click squish, heartbeat and hover jiggle nudge it by a pixel or two.
    let beat = a.beat_factor();
    let inset = (a.squish * a.squish * 3.0).max(beat * 2.0);
    let body = Rect::new(inset, -rad, w - 2.0 * inset, h + rad - inset);
    g.fill_rrect(body, rad, BLACK);
    g.stroke_rrect(
        Rect::new(body.x + 0.5, -rad, body.w - 1.0, body.h - 0.5),
        rad,
        Color::white(0.10),
        1.0,
    );
    if t > 0.05 && a.page == Page::Media && a.mstate.has_track() {
        // the panel takes a warm tint from the album art
        g.fill_rrect(body, rad, a.accent.fade(0.10 * t));
    }
    if beat > 0.01 {
        g.stroke_rrect(body, rad, Color(1.0, 0.4, 0.5, 0.85 * beat), 2.5);
    }
    if a.level > 0.01 && t < 0.05 {
        // music glow: the pill's rim and fill breathe with the output level
        g.fill_rrect(body, rad, a.accent.fade(0.03 + 0.15 * a.level));
        g.stroke_rrect(body, rad, a.accent.fade(0.2 + 0.7 * a.level), 1.0 + 2.2 * a.level);
    }
    if a.chat_streaming && t < 0.05 {
        let pulse = 0.5 + 0.5 * (a.spin_t * 3.5).sin();
        let glow_col = C_BAT;
        g.fill_rrect(body, rad, glow_col.fade((0.03 + 0.08 * pulse) * (1.0 - t)));
        g.stroke_rrect(
            Rect::new(body.x + 0.5, -rad, body.w - 1.0, body.h - 0.5),
            rad,
            glow_col.fade((0.35 + 0.45 * pulse) * (1.0 - t)),
            1.5 + 0.8 * pulse,
        );
    }

    if a.alerting() && t < 0.05 {
        let pulse = 0.5 + 0.5 * (a.eq_phase * 0.9).sin();
        // important hints get their own strong colour so they read from the corner of the eye
        let (tint, base) = match a.alert_msg {
            "Meeting" => (Color(0.20, 0.45, 1.0, 1.0), 0.55),
            "Battery low" => (Color(1.0, 0.50, 0.10, 1.0), 0.55),
            "Take a break" => (Color(0.12, 0.72, 0.62, 1.0), 0.50),
            _ => (a.accent, 0.25),
        };
        g.fill_rrect(Rect::new(0.0, -rad, w, h + rad), rad, tint.fade(base + (0.95 - base) * 0.5 * pulse));
    }
    let c_alpha = (1.0 - t * 3.2).clamp(0.0, 1.0);
    let e_alpha = ((t - 0.45) / 0.55).clamp(0.0, 1.0);
    if c_alpha > 0.0 {
        draw_collapsed(a, w, c_alpha);
    }
    if e_alpha > 0.0 {
        draw_expanded(a, e_alpha, hits);
    }
    if let Some(ct) = a.confetti_t() {
        draw_confetti(a, w, h, ct);
    }
}
/// Confetti burst from the pill (or panel) after the focus timer ends: pure
/// functions of time, so there is no particle state to keep.
fn draw_confetti(a: &App, w: f32, h: f32, t: f32) {
    let g = &a.gfx;
    const COLORS: [Color; 6] = [
        Color(1.0, 0.45, 0.5, 1.0),
        Color(1.0, 0.8, 0.3, 1.0),
        Color(0.5, 0.9, 0.6, 1.0),
        Color(0.45, 0.75, 1.0, 1.0),
        Color(0.8, 0.6, 1.0, 1.0),
        Color(1.0, 1.0, 1.0, 1.0),
    ];
    let fade = (1.0 - (t - 2.4).max(0.0) / 0.6).clamp(0.0, 1.0);
    for i in 0..40u32 {
        let r = |k: u32| {
            let mut x = (i + 1).wrapping_mul(2654435761).wrapping_add(k.wrapping_mul(40503));
            x ^= x >> 15;
            x = x.wrapping_mul(2246822519);
            x ^= x >> 13;
            (x % 1000) as f32 / 1000.0
        };
        let x = w * (0.2 + 0.6 * r(1)) + (r(2) - 0.5) * 150.0 * t;
        let y = h * 0.5 - (30.0 + 80.0 * r(3)) * t + 0.5 * 220.0 * t * t;
        if y > h + 6.0 || x < -4.0 || x > w + 4.0 {
            continue;
        }
        let spin = (t * (6.0 + 8.0 * r(4)) + r(5) * 6.0).sin().abs();
        let c = COLORS[(r(6) * 6.0) as usize % 6];
        g.fill_rrect(Rect::new(x, y, 2.0 + 3.0 * spin, 4.0 - 2.0 * spin), 0.5, c.fade(fade));
    }
}


// ------------------------------------------------------------- collapsed ----

fn draw_collapsed(a: &App, w: f32, ca: f32) {
    let g = &a.gfx;
    let f = &g.fonts;
    let m = &a.mstate;
    let ch = a.col_h();
    let timer = a.timer_running();

    if a.alerting() {
        let white = Color(1.0, 1.0, 1.0, ca);
        let amber = Color(1.0, 0.72, 0.2, ca);
        let mut label = match (a.alert_msg, a.alert_start) {
            ("Meeting", Some(start)) if !a.alert_label.is_empty() => {
                let mins = (start - crate::cal::now().0 + 59).div_euclid(60);
                format!("{} {}", a.alert_label, if mins > 0 { format!("{mins}m") } else { "now".to_string() })
            }
            _ => a.alert_msg.to_string(),
        };
        let mut col = white;
        if let Some(sw) = a.alert_swatch {
            let (cx, cy) = (w / 2.0 - 52.0, ch / 2.0);
            g.fill_circle(cx, cy, 7.0, Color::white(0.35 * ca));
            g.fill_circle(cx, cy, 6.0, Color(sw.0, sw.1, sw.2, ca));
        } else if a.alert_msg == "Battery low" {
            col = amber;
            if let Some(s) = sys::battery_info().filter(|b| !b.plugged).and_then(|b| b.secs_left) {
                label = format!("Battery low {}", if s >= 3600 { format!("{}h{:02}m", s / 3600, s % 3600 / 60) } else { format!("{}m", s / 60) });
            }
            g.icon('\u{E850}', Rect::new(w / 2.0 - 66.0, 0.0, 20.0, ch), &f.icon14, amber);
        } else if a.alert_msg == "Meeting" {
            g.icon('\u{E787}', Rect::new(16.0, 0.0, 20.0, ch), &f.icon14, white);
        } else if a.alert_msg == "Look away" {
            // a drawn eye
            let (ex, ey) = (w / 2.0 - 50.0, ch / 2.0);
            g.fill_rrect(Rect::new(ex - 8.0, ey - 5.0, 16.0, 10.0), 5.0, Color(1.0, 1.0, 1.0, ca));
            g.fill_circle(ex, ey, 3.4, Color(0.25, 0.55, 0.9, ca));
            g.fill_circle(ex, ey, 1.5, Color(0.0, 0.0, 0.0, ca));
        } else if a.alert_msg == "Drink water" {
            // a drawn water drop
            let (dx, dy) = (w / 2.0 - 52.0, ch / 2.0 + 1.0);
            g.fill_circle(dx, dy + 2.0, 5.0, Color(0.45, 0.8, 1.0, ca));
            g.fill_poly(&[(dx - 4.4, dy), (dx + 4.4, dy), (dx, dy - 8.0)], Color(0.45, 0.8, 1.0, ca));
        } else {
            g.icon('\u{E916}', Rect::new(w / 2.0 - 58.0, 0.0, 20.0, ch), &f.icon14, white);
        }
        let tx = if a.alert_msg == "Meeting" { 44.0 } else if a.alert_msg == "Battery low" { w / 2.0 - 42.0 } else { w / 2.0 - 34.0 };
        g.text(&label, Rect::new(tx, 0.0, (w - tx - 14.0).max(118.0), ch), &f.body, col, Align::Left);
        return;
    }

    if let (Some(t0), false) = (a.call_since, timer) {
        let mins = t0.elapsed().as_secs() / 60;
        let (cx, cy) = (w / 2.0 - 40.0, ch / 2.0);
        g.fill_circle(cx - 12.0, cy, 3.0, Color(1.0, 0.3, 0.3, ca));
        let muted = a.mic_muted;
        let red = Color(1.0, 0.3, 0.3, ca);
        g.icon('\u{E720}', Rect::new(cx - 6.0, 0.0, 18.0, ch), &f.icon14, if muted { red } else { T1.fade(ca) });
        if muted {
            g.fill_poly(&[(cx - 4.0, cy + 7.0), (cx - 2.0, cy + 8.5), (cx + 10.0, cy - 7.0), (cx + 8.0, cy - 8.5)], red);
        }
        let txt = if a.mic_muted { "Muted".to_string() } else if mins == 0 { "Mic on".to_string() } else { format!("Mic {}m", mins) };
        g.text(&txt, Rect::new(cx + 16.0, 0.0, 80.0, ch), &f.small, T1.fade(ca), Align::Left);
        return;
    }

    if m.has_track() {
        let ar = Rect::new(8.0, (ch - 22.0) / 2.0, 22.0, 22.0);
        match &a.art {
            Some(bmp) => g.draw_bitmap(bmp, ar, 6.0, ca),
            None => {
                g.fill_rrect(ar, 6.0, Color::white(0.14 * ca));
                g.text("\u{266A}", ar, &f.small, T2.fade(ca), Align::Center);
            }
        }
        // equaliser bars on the right
        let bx = w - 10.0 - 20.0;
        draw_bars(a, bx, ch / 2.0, ca);
        let mut right = bx - 8.0;
        if timer {
            let r = Rect::new(right - 50.0, 0.0, 50.0, ch);
            g.text(
                &fmt_time(a.timer_remaining_ms() as f64 / 1000.0),
                r,
                &f.small,
                T1.fade(ca),
                Align::Right,
            );
            right -= 58.0;
        }
        let title = if m.title.is_empty() {
            "Unknown"
        } else {
            &m.title
        };
        let label = if m.artist.is_empty() {
            title.to_string()
        } else {
            format!("{}  \u{00B7}  {}", title, m.artist)
        };
        g.text(
            &label,
            Rect::new(38.0, 0.0, (right - 38.0 - 4.0).max(10.0), ch),
            &f.small,
            T1.fade(ca),
            Align::Left,
        );
    } else if timer {
        g.icon('\u{E916}', Rect::new(w / 2.0 - 36.0, 0.0, 18.0, ch), &f.icon14, T1.fade(ca));
        g.text(
            &fmt_time(a.timer_remaining_ms() as f64 / 1000.0),
            Rect::new(w / 2.0 - 14.0, 0.0, 56.0, ch),
            &f.small,
            T1.fade(ca),
            Align::Left,
        );
    } else {
        let (_, _, _, _, hh, mm, _, _) = sys::local_time();
        let label = if a.cfg.clock_24h {
            format!("{:02}:{:02}", hh, mm)
        } else {
            let (h12, ampm) = match hh {
                0 => (12, "AM"),
                1..=11 => (hh, "AM"),
                12 => (12, "PM"),
                _ => (hh - 12, "PM"),
            };
            format!("{}:{:02} {}", h12, mm, ampm)
        };
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        let (_, _, day, wday, ..) = sys::local_time();
        // date | time, centred as a pair; the time is the bold part
        let (dw, tw) = (50.0, if a.cfg.clock_24h { 44.0 } else { 70.0 });
        let glance = a.glance();
        let gw = glance.as_ref().map_or(0.0, |(_, gw)| gw + 16.0);
        let x0 = (w - (dw + 15.0 + tw + gw)) / 2.0;
        g.text(&format!("{} {}", DAYS[(wday as usize).min(6)], day), Rect::new(x0, 0.0, dw, ch), &f.small, T3.fade(ca), Align::Right);
        g.fill_rrect(Rect::new(x0 + dw + 7.0, ch / 2.0 - 7.0, 1.0, 14.0), 0.5, Color::white(0.18 * ca));
        g.text(&label, Rect::new(x0 + dw + 15.0, 0.0, tw, ch), &f.menu, T1.fade(ca), Align::Left);
        if let Some((txt, gwid)) = glance {
            // optional glance: separator, then secondary-colour text (tasks get a small checkbox)
            let sx = x0 + dw + 15.0 + tw + 7.0;
            g.fill_rrect(Rect::new(sx, ch / 2.0 - 7.0, 1.0, 14.0), 0.5, Color::white(0.18 * ca));
            let mut gx = sx + 9.0;
            let kind = a.glance_kind();
            if kind == 1 {
                g.icon('\u{E73A}', Rect::new(gx, 0.0, 16.0, ch), &f.icon14, T2.fade(ca));
                gx += 19.0;
            }
            // the next meeting is dimmer than the other glances
            let gcol = if kind == 4 { Color(0.55, 0.53, 0.49, ca) } else { T2.fade(ca) };
            g.text(&txt, Rect::new(gx, 0.0, gwid, ch), &f.small, gcol, Align::Left);
        }
        if a.cycle_dots() {
            // page dots while cycling: the active page in blue, the others grey
            let active = a.cycle as usize % 4;
            for i in 0..4 {
                let c = if i == active { Color(DEFAULT_ACCENT.0, DEFAULT_ACCENT.1, DEFAULT_ACCENT.2, ca) } else { Color(0.267, 0.267, 0.267, ca) };
                g.fill_circle(w / 2.0 - 12.0 + i as f32 * 8.0, ch - 4.0, 2.0, c);
            }
        } else if a.cfg.day_line {
            let (_, _, _, _, hh, mm, _, _) = sys::local_time();
            if let Some(frac) = crate::app::day_fraction(hh, mm, a.cfg.day_start, a.cfg.day_end) {
                let (x, len, y) = (14.0, (w - 28.0).max(1.0), ch - 5.0);
                g.fill_rrect(Rect::new(x, y, len, 2.0), 1.0, Color(0.15, 0.15, 0.165, ca));
                g.fill_rrect(Rect::new(x, y, (len * frac).max(2.0), 2.0), 1.0, Color(0.55, 0.53, 0.49, ca));
            }
        }
        if a.chat_streaming {
            let pulse = 0.5 + 0.5 * (a.spin_t * 5.0).sin();
            let tint = C_BAT;
            let ix = w - 24.0;
            let ic_r = Rect::new(ix - 10.0, (ch - 16.0) / 2.0, 16.0, 16.0);
            g.fill_circle(ic_r.cx(), ic_r.cy(), 8.0, tint.fade(0.18 * pulse * ca));
            g.icon('\u{E950}', ic_r, &f.icon14, mix(tint, Color::white(1.0), pulse * 0.4).fade(ca));
            for d in 0..3 {
                let phase = a.spin_t * 6.5 - d as f32 * 0.8;
                let dy = (phase.sin() * 2.2).clamp(-2.5, 2.5);
                let dot_x = ix - 26.0 + d as f32 * 4.5;
                let dot_alpha = (0.35 + 0.55 * (0.5 + 0.5 * phase.sin())) * ca;
                g.fill_circle(dot_x, ch / 2.0 + dy, 1.4, tint.fade(dot_alpha));
            }
        }
    }

    if timer && a.cfg.idle_focus_bar {
        draw_focus_line(a, w, ch, ca);
    }
    if a.cfg.state_dot && !m.has_track() {
        draw_state_dot(a, ch, ca);
    }
    if let Some(k) = a.peek_k() {
        use crate::buddy::BuddyKind;
        if let Some(kind @ (BuddyKind::Robot | BuddyKind::Cat)) = BuddyKind::from_id(a.cfg.buddy) {
            crate::buddy::draw_peek(g, kind, w - 26.0, ch, k, a.peek_t());
        }
    }
}

/// The state glyph at the left of the idle and timer notch. Shape differs by state, not just colour.
fn draw_state_dot(a: &App, ch: f32, ca: f32) {
    let g = &a.gfx;
    let (cx, cy) = (13.0, ch / 2.0);
    let blue = Color(DEFAULT_ACCENT.0, DEFAULT_ACCENT.1, DEFAULT_ACCENT.2, ca);
    match a.dot_state() {
        DotState::Idle => g.fill_circle(cx, cy, 3.0, blue),
        DotState::Mic | DotState::MicMuted => {
            let c = if a.dot_state() == DotState::MicMuted { Color(1.0, 0.3, 0.3, ca) } else { blue };
            g.fill_circle(cx, cy, 4.2, c);
            g.fill_circle(cx, cy, 2.6, BLACK);
        }
        DotState::Battery => {
            let amber = Color(1.0, 0.72, 0.2, ca);
            g.fill_rrect(Rect::new(cx - 4.0, cy - 4.0, 8.0, 8.0), 1.8, amber);
            g.fill_rrect(Rect::new(cx - 0.55, cy - 2.6, 1.1, 3.2), 0.5, BLACK);
            g.fill_circle(cx, cy + 2.0, 0.6, BLACK);
        }
        DotState::Focus => {
            let done = focus_fraction(a.timer_total_s, a.timer_remaining_ms());
            let col = if a.break_mode { Color(0.12, 0.72, 0.62, ca) } else { blue };
            let steps = 28;
            let pt = |i: usize| {
                let ang = -std::f32::consts::FRAC_PI_2 + i as f32 / steps as f32 * std::f32::consts::TAU;
                (cx + 4.4 * ang.cos(), cy + 4.4 * ang.sin())
            };
            for i in 0..steps {
                g.line(pt(i), pt(i + 1), 1.6, Color(0.2, 0.2, 0.22, ca));
            }
            for i in 0..((done * steps as f32).round() as usize).min(steps) {
                g.line(pt(i), pt(i + 1), 1.8, col);
            }
        }
    }
}

/// How much of a timer session has passed, 0 to 1.
pub fn focus_fraction(total_s: u32, remaining_ms: i64) -> f32 {
    let total = (total_s as f32 * 1000.0).max(1.0);
    (1.0 - remaining_ms as f32 / total).clamp(0.0, 1.0)
}

/// A 2 px line along the inside bottom edge of the pill: how much of the focus session has passed.
fn draw_focus_line(a: &App, w: f32, ch: f32, ca: f32) {
    let g = &a.gfx;
    let done = focus_fraction(a.timer_total_s, a.timer_remaining_ms());
    let (x, len, y) = (14.0, (w - 28.0).max(1.0), ch - 5.0);
    g.fill_rrect(Rect::new(x, y, len, 2.0), 1.0, Color(0.15, 0.15, 0.165, ca));
    if done > 0.0 {
        // the existing blue; breaks use the teal of the break nudge
        let c = if a.break_mode { Color(0.12, 0.72, 0.62, ca) } else { Color(DEFAULT_ACCENT.0, DEFAULT_ACCENT.1, DEFAULT_ACCENT.2, ca) };
        g.fill_rrect(Rect::new(x, y, (len * done).max(2.0), 2.0), 1.0, c);
    }
}

fn draw_bars(a: &App, x: f32, cy: f32, ca: f32) {
    let g = &a.gfx;
    let playing = a.mstate.playing;
    let col = a.accent.fade(ca * if playing { 1.0 } else { 0.45 });
    for i in 0..4 {
        let hgt = if playing && a.cfg.eq_animation {
            4.0 + 10.0 * (0.5 + 0.5 * (a.eq_phase + i as f32 * 1.7).sin())
        } else if playing {
            [9.0, 13.0, 8.0, 11.0][i]
        } else {
            [5.0, 8.0, 6.0, 4.0][i]
        };
        let bx = x + i as f32 * 5.5;
        g.fill_rrect(Rect::new(bx, cy - hgt * 0.5, 3.0, hgt), 1.5, col);
    }
}

// -------------------------------------------------------------- expanded ----

fn hot(a: &App, act: Act) -> bool {
    a.hot == Some(act)
}

/// How hovered a control is, 0..1 (eases in and out instead of snapping).
fn hot_k(a: &App, act: Act) -> f32 {
    a.hover.iter().find(|(x, _)| *x == act).map_or(0.0, |(_, v)| *v)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t, a.3 + (b.3 - a.3) * t)
}

/// The hover squash of the collapsed pill, same keyframes as the website's notch:
/// wide and short, then narrow and tall, then back. `jiggle` runs 1 -> 0.
pub fn squash(jiggle: f32) -> (f32, f32) {
    let p = 1.0 - jiggle.clamp(0.0, 1.0);
    let (a, b, t) = if p < 0.3 {
        ((1.0, 1.0), (1.05, 0.92), p / 0.3)
    } else if p < 0.6 {
        ((1.05, 0.92), (0.98, 1.04), (p - 0.3) / 0.3)
    } else {
        ((0.98, 1.04), (1.0, 1.0), (p - 0.6) / 0.4)
    };
    let k = t * t * (3.0 - 2.0 * t);
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

pub const TAB_ITEMS: [(char, &str); 11] = [
    ('\u{E8D6}', "Media"),
    ('\u{E713}', "Controls"),
    ('\u{E787}', "Today"),
    ('\u{E821}', "Work"),
    ('\u{E950}', "Chat"),
    ('\u{E9D9}', "Stats"),
    ('\u{E77F}', "Clipboard"),
    ('\u{E8F1}', "Shelf"),
    ('\u{E71D}', "Apps"),
    ('\u{E7FC}', "Games"),
    ('\u{E7E8}', "Power"),
];

#[allow(dead_code)]
pub const TAB_LABELS: [&str; 11] = ["Media", "Controls", "Today", "Work", "Chat", "Stats", "Clipboard", "Shelf", "Apps", "Games", "Power"];

/// Where each tab sits in the top row, in the same order as `PAGES`.
pub fn tab_rects() -> Vec<Rect> {
    const TAB_W: f32 = 36.0;
    let n = TAB_ITEMS.len();
    let total = n as f32 * TAB_W;
    let (left, right) = (46.0, EXP_W - 46.0);
    let gap = (right - left - total) / (n as f32 - 1.0);
    let mut x = left;
    (0..n)
        .map(|_| {
            let r = Rect::new(x, 8.0, TAB_W, 28.0);
            x += TAB_W + gap;
            r
        })
        .collect()
}

fn draw_expanded(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;

    // top row: pin, labelled tabs, collapse. The pages sit below it.
    let pin = Rect::new(12.0, 8.0, 28.0, 28.0);
    hits.push(Hit { r: pin, act: Act::Pin });
    let pc = if a.pinned { a.accent } else if hot(a, Act::Pin) { T1 } else { T3 };
    g.icon('\u{E718}', pin, &f.icon14, pc.fade(ca));

    let chev = Rect::new(EXP_W - 40.0, 8.0, 28.0, 28.0);
    hits.push(Hit { r: chev, act: Act::Collapse });
    g.icon('\u{E70E}', chev, &f.icon14, (if hot(a, Act::Collapse) { T1 } else { T3 }).fade(ca));

    // the active-tab highlight slides from tab to tab; each icon takes the accent
    // colour in proportion to how much of the highlight is behind it
    let rects = tab_rects();
    let pill = Rect::new(a.tab_x, 8.0, a.tab_w, 28.0);
    g.fill_rrect(pill, 14.0, a.accent.fade(0.20 * ca));
    for (i, p) in PAGES.iter().enumerate() {
        let r = rects[i];
        let act = Act::Tab(*p);
        hits.push(Hit { r, act });
        let k = hot_k(a, act);
        if a.page != *p && k > 0.01 {
            g.fill_rrect(r, 14.0, Color::white(0.08 * k * ca));
        }
        let covered = ((pill.x + pill.w).min(r.x + r.w) - pill.x.max(r.x)).max(0.0) / r.w;
        let col = mix(mix(T3, T1, k), a.accent, covered);
        g.icon(TAB_ITEMS[i].0, r, &f.icon14, col.fade(ca));
    }

    // the page itself, moved below the tab row; its hit boxes move with it
    let start = hits.len();
    let draw_page = |p: Page, ca: f32, hits: &mut Vec<Hit>| match p {
        Page::Media => page_media(a, ca, hits),
        Page::Controls => page_controls(a, ca, hits),
        Page::Clipboard => page_clipboard(a, ca, hits),
        Page::Apps => page_apps(a, ca, hits),
        Page::Shelf => page_shelf(a, ca, hits),
        Page::Tasks => page_tasks(a, ca, hits),
        Page::Work => page_work(a, ca, hits),
        Page::Chat => page_chat(a, ca, hits),
        Page::Stats => page_stats(a, ca),
        Page::Power => page_power(a, ca, hits),
        Page::Game => {
            let (ox, oy) = ((EXP_W - crate::game::W) / 2.0, 14.0);
            if a.hub.is_playing() {
                hits.push(Hit { r: Rect::new(ox, oy, crate::game::W, crate::game::H), act: Act::GameClick });
                a.hub.draw(g, ox, oy, ca);
                let r = Rect::new(ox + crate::game::W / 2.0 - 36.0, oy + 3.0, 72.0, 18.0);
                hits.push(Hit { r, act: Act::GameBack });
                g.fill_rrect(r, 9.0, (if hot(a, Act::GameBack) { HOVER } else { Color(0.0, 0.0, 0.0, 0.35) }).fade(ca));
                g.text("\u{2039} Games", r, &f.tiny, T1.fade(ca), Align::Center);
            } else {
                let hot_tile = (0..4).find(|i| hot(a, Act::GamePick(*i)));
                a.hub.draw_hub(g, ox, oy, ca, hot_tile);
                for (i, r) in crate::games::Hub::tiles(ox, oy).iter().enumerate() {
                    hits.push(Hit { r: *r, act: Act::GamePick(i) });
                }
            }
        }
    };
    if a.page_t < 1.0 {
        // cross-fade: the old page drifts up and fades out (not clickable), the new one rises in
        let k = 1.0 - (1.0 - a.page_t).powi(3);
        let mut gone = Vec::new();
        g.shifted(0.0, CONTENT_Y - 6.0 * k, || draw_page(a.prev_page, ca * (1.0 - k), &mut gone));
        g.shifted(0.0, CONTENT_Y + 8.0 * (1.0 - k), || draw_page(a.page, ca * k, hits));
    } else {
        g.shifted(0.0, CONTENT_Y, || draw_page(a.page, ca, hits));
    }
    for h in &mut hits[start..] {
        h.r.y += CONTENT_Y;
    }

    // Floating title span tooltip for hovered tab (drawn on top of page content)
    for (i, p) in PAGES.iter().enumerate() {
        let act = Act::Tab(*p);
        let k = hot_k(a, act);
        if k > 0.05 {
            let r = rects[i];
            let label = TAB_ITEMS[i].1;
            let tw = (label.len() as f32 * 7.5 + 20.0).max(52.0);
            let tip_r = Rect::new(
                (r.cx() - tw / 2.0).clamp(12.0, EXP_W - tw - 12.0),
                38.0,
                tw,
                22.0,
            );
            g.fill_rrect(tip_r, 11.0, BLACK.fade(0.92 * k * ca));
            g.fill_rrect(tip_r, 11.0, Color::white(0.14 * k * ca));
            g.text(label, tip_r, &f.small, T1.fade(k * ca), Align::Center);
            break;
        }
    }
}

// ----------------------------------------------------------------- media ----

fn page_media(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    // a Previous / Next press that is still loading: keep showing the old track
    let loading = a.skip_loading();
    let m = loading.map_or(&a.mstate, |s| &s.snap);
    let art = loading.map_or(&a.art, |s| &s.art);

    if !m.has_track() {
        g.icon(
            '\u{E768}',
            Rect::new(0.0, 56.0, EXP_W, 40.0),
            &f.icon22,
            T3.fade(ca),
        );
        g.text(
            "Nothing playing",
            Rect::new(0.0, 98.0, EXP_W, 24.0),
            &f.body,
            T2.fade(ca),
            Align::Center,
        );
        g.text(
            "Start music or a video and it shows up here",
            Rect::new(0.0, 122.0, EXP_W, 20.0),
            &f.small,
            T3.fade(ca),
            Align::Center,
        );
        return;
    }

    // artwork
    let ar = Rect::new(28.0, 22.0, 132.0, 132.0);
    match art {
        Some(bmp) => g.draw_bitmap(bmp, ar, 16.0, ca),
        None => {
            g.fill_rrect(ar, 16.0, Color::white(0.10 * ca));
            g.text("\u{266A}", ar, &f.big, T3.fade(ca), Align::Center);
        }
    }
    if loading.is_some() {
        g.fill_rrect(ar, 16.0, Color(0.0, 0.0, 0.0, 0.5 * ca));
        spinner(g, ar.cx(), ar.cy(), 18.0, a.spin_t, a.accent, ca);
    }

    let tx = 182.0;
    let tw = EXP_W - tx - 56.0;
    let title = if m.title.is_empty() {
        "Unknown title"
    } else {
        &m.title
    };
    g.text(
        title,
        Rect::new(tx, 22.0, tw, 26.0),
        &f.title,
        T1.fade(ca),
        Align::Left,
    );
    let sub = match loading {
        Some(s) if s.next => "Loading next track\u{2026}",
        Some(_) => "Loading previous track\u{2026}",
        None if m.artist.is_empty() => &m.album,
        None => &m.artist,
    };
    g.text(
        sub,
        Rect::new(tx, 49.0, tw, 22.0),
        &f.body,
        (if loading.is_some() { a.accent } else { T2 }).fade(ca),
        Align::Left,
    );
    if !m.app.is_empty() {
        g.text(
            &m.app,
            Rect::new(tx, 72.0, tw, 18.0),
            &f.tiny,
            T3.fade(ca),
            Align::Left,
        );
    }

    // progress
    let (pos, dur) = if loading.is_some() { (0.0, 0.0) } else { m.progress(sys::filetime_now()) };
    let frac = match a.seek_frac {
        Some(fr) => fr,
        None if dur > 0.0 => (pos / dur) as f32,
        None => 0.0,
    };
    let shown = if a.seek_frac.is_some() {
        frac as f64 * dur
    } else {
        pos
    };
    let bar = Rect::new(tx, 104.0, EXP_W - tx - 28.0, 18.0);
    slider(a, bar, frac, Act::Seek, a.accent, ca, hits, dur > 0.0 && m.can_seek);
    g.text(
        &fmt_time(shown),
        Rect::new(tx, 122.0, 70.0, 16.0),
        &f.tiny,
        T3.fade(ca),
        Align::Left,
    );
    if dur > 0.0 {
        g.text(
            &fmt_time(dur),
            Rect::new(bar.x + bar.w - 70.0, 122.0, 70.0, 16.0),
            &f.tiny,
            T3.fade(ca),
            Align::Right,
        );
    }

    // transport
    let cx = tx + (EXP_W - tx - 28.0) / 2.0;
    let cy = 168.0;
    let prev = Rect::new(cx - 92.0, cy - 20.0, 40.0, 40.0);
    let play = Rect::new(cx - 26.0, cy - 26.0, 52.0, 52.0);
    let next = Rect::new(cx + 52.0, cy - 20.0, 40.0, 40.0);
    hits.push(Hit {
        r: prev,
        act: Act::Prev,
    });
    hits.push(Hit {
        r: play,
        act: Act::PlayPause,
    });
    hits.push(Hit {
        r: next,
        act: Act::Next,
    });
    for (r, act, glyph, ok) in [(prev, Act::Prev, '\u{E892}', m.can_prev), (next, Act::Next, '\u{E893}', m.can_next)] {
        if hot(a, act) {
            g.fill_rrect(r, 20.0, Color::white(0.10 * ca));
        }
        g.icon(glyph, r, &f.icon18, (if ok { T1 } else { T3 }).fade(ca));
    }
    let big = if hot(a, Act::PlayPause) { 1.0 } else { 0.92 };
    g.fill_circle(play.cx(), play.cy(), 24.0, Color::white(big * ca));
    let glyph = if m.playing { '\u{E769}' } else { '\u{E768}' };
    g.icon(glyph, play, &f.icon22, Color(0.0, 0.0, 0.0, ca));
}

// -------------------------------------------------------------- controls ----

/// Shared Controls-style borderless matte card: tinted circular icon badge on the left,
/// stacked primary title (`T1`) + muted subtitle (`T3`), and optional trailing glyph.
#[allow(clippy::too_many_arguments)]
fn card_tile(
    a: &App,
    ca: f32,
    hits: &mut Vec<Hit>,
    r: Rect,
    act: Option<Act>,
    glyph: char,
    tint: Color,
    title: &str,
    title_col: Color,
    sub: &str,
    sub_col: Color,
    trailing: Option<(char, Color)>,
    focused: bool,
    on: bool,
) {
    let g = &a.gfx;
    let f = &g.fonts;
    let k = if let Some(ac) = act {
        hits.push(Hit { r, act: ac });
        hot_k(a, ac)
    } else {
        0.0
    };
    if on {
        g.fill_rrect(r, 18.0, a.accent.fade(ca));
        if k > 0.01 {
            g.fill_rrect(r, 18.0, Color::white(0.10 * k * ca));
        }
    } else if focused {
        g.fill_rrect(r, 18.0, HOVER.fade(ca));
        g.fill_rrect(r, 18.0, tint.fade(0.14 * ca));
    } else {
        g.fill_rrect(r, 18.0, Color::white((0.09 + 0.08 * k) * ca));
    }

    let ic = Rect::new(r.x + 12.0, r.y + (r.h - 34.0) / 2.0, 34.0, 34.0);
    g.fill_circle(
        ic.cx(),
        ic.cy(),
        17.0,
        (if on { Color(0.0, 0.0, 0.0, 0.14) } else { tint.fade(0.16) }).fade(ca),
    );
    g.icon(glyph, ic, &f.icon14, (if on { BLACK } else { tint }).fade(ca));

    let (tc, sc) = if on {
        (BLACK, Color(0.0, 0.0, 0.0, 0.65))
    } else {
        (title_col, sub_col)
    };
    let right_pad = if trailing.is_some() { 34.0 } else { 10.0 };
    let tw = (r.w - 56.0 - right_pad).max(40.0);
    let ty = r.y + (r.h - 43.0) / 2.0;
    g.text(title, Rect::new(r.x + 56.0, ty, tw, 22.0), &f.body, tc.fade(ca), Align::Left);
    g.text(sub, Rect::new(r.x + 56.0, ty + 23.0, tw, 18.0), &f.tiny, sc.fade(ca), Align::Left);

    if let Some((tg, tcol)) = trailing {
        g.icon(tg, Rect::new(r.x + r.w - 30.0, r.y, 20.0, r.h), &f.icon14, tcol.fade(ca));
    }
}

/// One rounded tile: round icon, title, small state line. `on` fills it with the accent.
#[allow(clippy::too_many_arguments)]
fn tile(a: &App, ca: f32, hits: &mut Vec<Hit>, r: Rect, act: Act, glyph: char, title: &str, sub: &str, on: bool) {
    let tint = match glyph {
        '\u{E722}' => Color(0.98, 0.45, 0.55, 1.0),
        '\u{E7F4}' => C_MEM,
        '\u{E767}' => C_UP,
        '\u{E774}' => C_CPU,
        '\u{E83F}' => C_BAT,
        _ => T1,
    };
    card_tile(a, ca, hits, r, Some(act), glyph, tint, title, T1, sub, T3, None, false, on);
}

fn page_controls(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    // Wi-Fi, Bluetooth, sound and brightness live in the top bar and Windows' own
    // quick settings, so this page is shortcuts to the screenshot tool and the
    // Settings pages people actually go looking for.
    let (w, h, gap) = (270.0, 54.0, 12.0);
    let (x0, y0) = (24.0, 6.0);
    let at = |col: f32, row: f32| Rect::new(x0 + col * (w + gap), y0 + row * (h + 8.0), w, h);
    tile(a, ca, hits, at(0.0, 0.0), Act::Quick(2), '\u{E722}', "Screenshot", "Snip a region", false);
    tile(a, ca, hits, at(1.0, 0.0), Act::OpenSettings(0), '\u{E7F4}', "Display", "Resolution, scale, night light", false);
    tile(a, ca, hits, at(0.0, 1.0), Act::OpenSettings(1), '\u{E767}', "Sound", "Output, devices, volume mixer", false);
    tile(a, ca, hits, at(1.0, 1.0), Act::OpenSettings(2), '\u{E774}', "Network", "Wi-Fi, Ethernet, VPN", false);
    tile(a, ca, hits, at(0.0, 2.0), Act::OpenSettings(3), '\u{E83F}', "Power & battery", "Sleep, plan, battery saver", false);
    tile(a, ca, hits, at(1.0, 2.0), Act::OpenSettings(4), '\u{E713}', "All settings", "Open Windows Settings", false);
}

// ----------------------------------------------------------------- power ----

fn page_power(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    let armed = |k: u8| matches!(a.power_confirm, Some((p, t)) if p == k && t.elapsed().as_secs_f32() < 3.5);
    let items: [(char, &str, Act, u8); 4] = [
        ('\u{E72E}', "Lock", Act::Quick(3), 0),
        ('\u{E708}', "Sleep", Act::Quick(4), 0),
        ('\u{E777}', "Restart", Act::PowerRestart, 1),
        ('\u{E7E8}', "Shut down", Act::PowerShutdown, 2),
    ];
    let (w, gap) = (129.0, 12.0);
    for (i, (glyph, label, act, k)) in items.iter().enumerate() {
        let r = Rect::new(24.0 + i as f32 * (w + gap), 28.0, w, 130.0);
        hits.push(Hit { r, act: *act });
        let danger = *k != 0 && armed(*k);
        // colour says how serious it is: calm blue and violet, amber for restart, red for shut down
        let tint = [C_CPU, C_UP, C_MEM, Color(1.0, 0.42, 0.45, 1.0)][i];
        let bg = if danger { Color(0.85, 0.25, 0.28, 1.0) } else if hot(a, *act) { HOVER } else { SURF };
        g.fill_rrect(r, 20.0, bg.fade(ca));
        if !danger {
            g.fill_rrect(r, 20.0, tint.fade((if hot(a, *act) { 0.14 } else { 0.07 }) * ca));
            g.fill_circle(r.cx(), r.y + 42.0, 24.0, tint.fade(0.16 * ca));
        }
        g.icon(*glyph, Rect::new(r.x, r.y + 22.0, r.w, 40.0), &f.icon22, (if danger { Color::white(1.0) } else { tint }).fade(ca));
        g.text(label, Rect::new(r.x, r.y + 70.0, r.w, 24.0), &f.body, (if danger { Color::white(1.0) } else { T1 }).fade(ca), Align::Center);
        if *k != 0 {
            g.text(
                if danger { "Click again to confirm" } else { "Asks to confirm" },
                Rect::new(r.x + 4.0, r.y + 96.0, r.w - 8.0, 20.0),
                &f.tiny,
                (if danger { Color::white(0.95) } else { T3 }).fade(ca),
                Align::Center,
            );
        }
    }
}

// ----------------------------------------------------------------- stats ----

/// Per-metric colours, so each stat reads at a glance.
const C_CPU: Color = Color(0.30, 0.78, 0.98, 1.0);
const C_MEM: Color = Color(0.98, 0.70, 0.20, 1.0);
const C_BAT: Color = Color(0.40, 0.85, 0.50, 1.0);
const C_UP: Color = Color(0.72, 0.58, 1.0, 1.0);
const C_FOC: Color = Color(1.0, 0.58, 0.42, 1.0);

/// One inset tile: coloured icon + dim label, bold value, thin bar underneath.
#[allow(clippy::too_many_arguments)]
fn stat_tile(g: &Gfx, r: Rect, glyph: char, label: &str, value: &str, note: &str, frac: Option<f32>, col: Color, ca: f32) {
    let f = &g.fonts;
    g.fill_rrect(r, 20.0, SURF.fade(ca));
    g.icon(glyph, Rect::new(r.x + 14.0, r.y + 12.0, 22.0, 22.0), &f.icon14, col.fade(ca));
    g.text(label, Rect::new(r.x + 42.0, r.y + 12.0, 120.0, 22.0), &f.small, T3.fade(ca), Align::Left);
    g.text(note, Rect::new(r.x + r.w - 150.0, r.y + 12.0, 136.0, 22.0), &f.tiny, T3.fade(ca), Align::Right);
    g.text(value, Rect::new(r.x + 16.0, r.y + 36.0, r.w - 32.0, 34.0), &f.big, T1.fade(ca), Align::Left);
    if let Some(fr) = frac {
        let bar = Rect::new(r.x + 16.0, r.y + r.h - 18.0, r.w - 32.0, 5.0);
        g.fill_rrect(bar, 2.5, Color::white(0.10 * ca));
        g.fill_rrect(Rect::new(bar.x, bar.y, (bar.w * fr.clamp(0.0, 1.0)).max(5.0), bar.h), 2.5, col.fade(ca));
    }
}

fn page_stats(a: &App, ca: f32) {
    let g = &a.gfx;
    let (w, h, gap) = (280.0, 92.0, 12.0);
    let at = |c: f32, r: f32| Rect::new(14.0 + c * (w + gap), 6.0 + r * (h + gap), w, h);
    stat_tile(g, at(0.0, 0.0), '\u{E9D9}', "CPU", &format!("{}%", a.stat_cpu), "", Some(a.bar_cpu), C_CPU, ca);
    stat_tile(g, at(1.0, 0.0), '\u{E964}', "Memory", &format!("{}%", a.stat_mem), "", Some(a.bar_mem), C_MEM, ca);
    // CPU history beside the value
    let r = at(0.0, 0.0);
    let n = a.stat_hist.len();
    for (i, v) in a.stat_hist.iter().enumerate() {
        let bh = 2.0 + v.clamp(0.0, 1.0) * 22.0;
        g.fill_rrect(Rect::new(r.x + r.w - 16.0 - (n - i) as f32 * 4.6, r.y + 62.0 - bh, 3.0, bh), 1.5, C_CPU.fade(((i + 1) as f32 / n.max(1) as f32) * 0.7 * ca));
    }
    match sys::battery_info() {
        Some(b) => {
            let state = if b.charging { "Charging" } else if b.plugged { "Plugged in" } else { "On battery" };
            let col = if b.percent <= 20 && !b.charging { Color(1.0, 0.45, 0.4, 1.0) } else { C_BAT };
            stat_tile(g, at(0.0, 1.0), '\u{E83F}', "Battery", &format!("{}%", b.percent), state, Some(b.percent as f32 / 100.0), col, ca);
        }
        None => stat_tile(g, at(0.0, 1.0), '\u{E83F}', "Power", "Plugged in", "No battery", None, C_BAT, ca),
    }
    let up = sys::uptime_secs();
    let (d, hh, m) = (up / 86400, (up % 86400) / 3600, (up % 3600) / 60);
    let text = if d > 0 { format!("{d}d {hh}h {m}m") } else { format!("{hh}h {m}m") };
    stat_tile(g, at(1.0, 1.0), '\u{E916}', "Uptime", &text, "", None, C_UP, ca);
}

// ------------------------------------------------------------- clipboard ----

fn page_clipboard(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    let view = a.clip_view();

    let total = a.pins.len() + a.clips.len();
    let head = if total == 0 {
        "Clipboard".to_string()
    } else {
        format!("Clipboard  \u{00B7}  {}", total)
    };
    g.text(&head, Rect::new(28.0, 12.0, 200.0, 24.0), &f.small, C_MEM.fade(ca), Align::Left);
    if !a.clips.is_empty() {
        let r = Rect::new(EXP_W - 112.0, 12.0, 64.0, 24.0);
        hits.push(Hit { r, act: Act::ClipClear });
        if hot(a, Act::ClipClear) {
            g.fill_rrect(r, 12.0, Color::white(0.10 * ca));
        }
        g.text("Clear", r, &f.small, C_PINK.fade(ca), Align::Center);
    }
    // search chip: click, type to filter, Enter keeps it, Esc clears it
    let sr = Rect::new(EXP_W - 252.0, 12.0, 132.0, 24.0);
    let typing = a.entry_search;
    if typing || !a.clip_filter.is_empty() {
        let mut t = a.clip_filter.clone();
        if typing {
            t.push('\u{2502}');
        }
        g.fill_rrect(sr, 12.0, Color::white(0.14 * ca));
        g.text(&t, Rect::new(sr.x + 10.0, sr.y, sr.w - 30.0, sr.h), &f.small, T1.fade(ca), Align::Left);
        if !typing {
            let x = Rect::new(sr.x + sr.w - 24.0, sr.y, 24.0, sr.h);
            hits.push(Hit { r: sr, act: Act::ClipSearch });
            hits.push(Hit { r: x, act: Act::ClipSearchClear });
            g.icon('\u{E711}', x, &f.icon14, (if hot(a, Act::ClipSearchClear) { T1 } else { T2 }).fade(ca));
        }
    } else {
        hits.push(Hit { r: sr, act: Act::ClipSearch });
        if hot(a, Act::ClipSearch) {
            g.fill_rrect(sr, 12.0, Color::white(0.10 * ca));
        }
        g.icon('\u{E721}', Rect::new(sr.x + 4.0, sr.y, 24.0, sr.h), &f.icon14, T3.fade(ca));
        g.text("Search", Rect::new(sr.x + 30.0, sr.y, sr.w - 34.0, sr.h), &f.small, T3.fade(ca), Align::Left);
    }

    if view.is_empty() {
        let (l1, l2) = if a.clip_filter.is_empty() {
            ("Copy something and it will appear here.", "Click an item to paste it. Click the star to pin it as a saved snippet.")
        } else {
            ("No matches.", "Press Esc in the search box to clear the filter.")
        };
        g.text(l1, Rect::new(0.0, 80.0, EXP_W, 24.0), &f.body, T3.fade(ca), Align::Center);
        g.text(l2, Rect::new(0.0, 104.0, EXP_W, 20.0), &f.small, T3.fade(ca * 0.8), Align::Center);
        return;
    }

    for row in 0..CLIP_ROWS {
        let idx = a.clip_scroll + row;
        let Some((text, pinned)) = view.get(idx) else { break };
        let r = Rect::new(24.0, 40.0 + row as f32 * 31.0, EXP_W - 48.0, 28.0);
        let act = Act::ClipItem(idx);
        hits.push(Hit { r, act });
        let pin = Rect::new(r.x + r.w - 32.0, r.y, 30.0, r.h);
        let pact = Act::ClipPin(idx);
        hits.push(Hit { r: pin, act: pact });
        let (kind, kc) = clip_kind(text);
        if hot(a, act) || hot(a, pact) {
            g.fill_rrect(r, 10.0, Color::white(0.12 * ca));
        } else {
            g.fill_rrect(r, 10.0, Color::white(0.04 * ca));
        }
        g.fill_rrect(r, 10.0, kc.fade(0.06 * ca));
        // a round chip in the colour of the kind of thing copied
        let chip = Rect::new(r.x + 6.0, r.y + 3.0, 22.0, 22.0);
        g.fill_circle(chip.cx(), chip.cy(), 11.0, kc.fade(0.20 * ca));
        g.icon(kind, chip, &f.icon14, kc.fade(ca));
        g.text(&preview(text), Rect::new(r.x + 38.0, r.y, r.w - 76.0, r.h), &f.body, T1.fade(ca), Align::Left);
        let (glyph, col) = if *pinned {
            ('\u{E735}', C_GOLD)
        } else if hot(a, pact) {
            ('\u{E734}', T1)
        } else {
            ('\u{E734}', T3)
        };
        g.icon(glyph, pin, &f.icon14, col.fade(ca * if *pinned || hot(a, act) || hot(a, pact) { 1.0 } else { 0.6 }));
    }

    // scroll bar: the thumb's size shows how much of the list is on screen, its place where you are
    if view.len() > CLIP_ROWS {
        let (top, h) = (40.0, CLIP_ROWS as f32 * 31.0 - 3.0);
        let track = Rect::new(EXP_W - 17.0, top, 4.0, h);
        g.fill_rrect(track, 2.0, Color::white(0.07 * ca));
        let thumb_h = (h * CLIP_ROWS as f32 / view.len() as f32).max(18.0);
        let frac = a.clip_scroll as f32 / (view.len() - CLIP_ROWS) as f32;
        g.fill_rrect(Rect::new(track.x, top + (h - thumb_h) * frac.clamp(0.0, 1.0), 4.0, thumb_h), 2.0, C_MEM.fade(0.8 * ca));
    }
}

const C_GOLD: Color = Color(1.0, 0.82, 0.28, 1.0);
const C_PINK: Color = Color(1.0, 0.50, 0.62, 1.0);

/// What kind of thing was copied: an icon and a colour for its row.
fn clip_kind(s: &str) -> (char, Color) {
    match clip_kind_of(s) {
        ClipKind::Link => ('\u{E71B}', C_CPU),
        ClipKind::Mail => ('\u{E715}', C_BAT),
        ClipKind::Path => ('\u{E8B7}', C_MEM),
        ClipKind::Code => ('\u{E943}', C_PINK),
        ClipKind::Text => ('\u{E8A5}', C_UP),
    }
}

// ------------------------------------------------------------------ apps ----

fn spinner(g: &Gfx, cx: f32, cy: f32, r: f32, t: f32, col: Color, ca: f32) {
    for i in 0..10 {
        let ang = t * 6.0 - i as f32 * 0.55;
        g.fill_circle(cx + r * ang.cos(), cy + r * ang.sin(), 3.4 - i as f32 * 0.25, col.fade((1.0 - i as f32 / 10.0) * ca));
    }
}

fn pinned_app_sub(target: &str) -> &'static str {
    if target.starts_with("http://") || target.starts_with("https://") {
        "Web shortcut"
    } else if target.starts_with("ms-settings:") {
        "Windows settings"
    } else if target.starts_with("shell:AppsFolder") {
        "Pinned app"
    } else {
        "Pinned shortcut"
    }
}

fn draw_craft_tile(a: &App, idx: usize, r: Rect, ca: f32, hits: &mut Vec<Hit>) {
    use crate::craft::{SPECS, State};
    let g = &a.gfx;
    let f = &g.fonts;
    let spec = &SPECS[idx];
    let st = &a.craft_states[idx];
    let act = Act::CraftClick(idx);
    let wact = Act::CraftWeb(idx);
    let installing = matches!(st, State::Installing { .. });
    let show_web_btn = matches!(st, State::Installed { .. } | State::Failed(_));

    if !installing {
        hits.push(Hit { r, act });
    }
    let web_r = Rect::new(r.x + r.w - 34.0, r.y + (r.h - 24.0) / 2.0, 24.0, 24.0);
    let over_main = !installing && hot(a, act);
    if show_web_btn && (over_main || hot(a, wact) || matches!(st, State::Failed(_))) {
        hits.push(Hit { r: web_r, act: wact });
    }

    let k = if installing {
        0.35
    } else {
        hot_k(a, act).max(hot_k(a, wact))
    };
    g.fill_rrect(r, 18.0, Color::white((0.09 + 0.08 * k) * ca));

    // Left 34x34 tinted circular icon badge (Controls Tile Design System)
    let tint = spec.tint;
    let ic = Rect::new(r.x + 12.0, r.y + (r.h - 34.0) / 2.0, 34.0, 34.0);
    g.fill_circle(ic.cx(), ic.cy(), 17.0, tint.fade(0.16 * ca));
    if installing {
        spinner(g, ic.cx(), ic.cy(), 9.0, a.spin_t, tint, ca);
    } else if let Some(bmp) = a.craft_icons[idx].as_ref() {
        g.draw_bitmap(bmp, Rect::new(ic.cx() - 11.0, ic.cy() - 11.0, 22.0, 22.0), 4.0, ca);
    } else {
        g.icon(spec.glyph, ic, &f.icon14, tint.fade(ca));
    }

    // Subtitle & semantic color per download/install state
    let hovered = over_main && !hot(a, wact);
    let (sub, sub_col): (String, Color) = match st {
        State::NotInstalled => {
            if hovered {
                ("Click to download portable app".into(), tint)
            } else {
                (format!("{}  \u{00B7}  Get", spec.tagline), T3)
            }
        }
        State::Installing { step, pct } => match pct {
            Some(p) => (format!("{step}  \u{00B7}  {p}%"), tint),
            None => (step.clone(), tint),
        },
        State::Installed { version, .. } => {
            if hot(a, wact) {
                ("Open official project page".into(), C_CPU)
            } else if hovered {
                ("Installed  \u{00B7}  Click to launch".into(), C_BAT)
            } else if let Some(v) = version {
                (format!("{}  \u{00B7}  {v}", spec.tagline), T3)
            } else {
                (format!("{}  \u{00B7}  Ready", spec.tagline), T3)
            }
        }
        State::Failed(err) => {
            if hot(a, wact) {
                ("Open download page in browser".into(), C_CPU)
            } else {
                (format!("{err}  \u{00B7}  Retry"), Color(0.98, 0.45, 0.55, 1.0))
            }
        }
    };

    let tw = (r.w - 56.0 - 38.0).max(40.0);
    let ty = r.y + (r.h - 43.0) / 2.0;
    g.text(spec.name, Rect::new(r.x + 56.0, ty, tw, 22.0), &f.body, T1.fade(ca), Align::Left);
    g.text(&sub, Rect::new(r.x + 56.0, ty + 23.0, tw, 18.0), &f.tiny, sub_col.fade(ca), Align::Left);

    // Trailing action / status glyph inside the card
    match st {
        State::NotInstalled => {
            let col = if hovered { tint } else { T2 };
            g.icon('\u{E896}', web_r, &f.icon14, col.fade(ca));
        }
        State::Installing { .. } => {}
        State::Installed { .. } => {
            if over_main || hot(a, wact) {
                g.fill_circle(
                    web_r.cx(),
                    web_r.cy(),
                    12.0,
                    Color::white((if hot(a, wact) { 0.16 } else { 0.08 }) * ca),
                );
                g.icon(
                    '\u{E71B}',
                    web_r,
                    &f.icon14,
                    (if hot(a, wact) { C_CPU } else { T2 }).fade(ca),
                );
            } else {
                g.icon('\u{E8A7}', web_r, &f.icon14, T3.fade(0.75 * ca));
            }
        }
        State::Failed(_) => {
            g.fill_circle(
                web_r.cx(),
                web_r.cy(),
                12.0,
                Color::white((if hot(a, wact) { 0.16 } else { 0.08 }) * ca),
            );
            g.icon(
                '\u{E71B}',
                web_r,
                &f.icon14,
                (if hot(a, wact) { C_CPU } else { T2 }).fade(ca),
            );
        }
    }
}

fn draw_pinned_tile(a: &App, i: usize, r: Rect, ca: f32, hits: &mut Vec<Hit>) {
    let Some(e) = a.apps.get(i) else { return };
    let g = &a.gfx;
    let f = &g.fonts;
    let act = Act::Launch(i);
    let xact = Act::AppUnpin(i);
    hits.push(Hit { r, act });

    let xr = Rect::new(r.x + r.w - 34.0, r.y + (r.h - 24.0) / 2.0, 24.0, 24.0);
    let over = hot(a, act) || hot(a, xact);
    if over {
        hits.push(Hit { r: xr, act: xact });
    }

    let k = hot_k(a, act).max(hot_k(a, xact));
    g.fill_rrect(r, 18.0, Color::white((0.09 + 0.08 * k) * ca));

    let hue = name_color(&e.name);
    let ic = Rect::new(r.x + 12.0, r.y + (r.h - 34.0) / 2.0, 34.0, 34.0);
    g.fill_circle(ic.cx(), ic.cy(), 17.0, hue.fade(0.16 * ca));
    match a.app_icons.get(i).and_then(|b| b.as_ref()) {
        Some(bmp) => g.draw_bitmap(bmp, Rect::new(ic.cx() - 11.0, ic.cy() - 11.0, 22.0, 22.0), 4.0, ca),
        None => {
            let letter: String = e.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            g.text(&letter, ic, &f.body, hue.fade(ca), Align::Center);
        }
    }

    let sub = if hot(a, xact) {
        "Unpin from Apps"
    } else if over {
        "Click to launch"
    } else {
        pinned_app_sub(&e.target)
    };
    let sub_col = if hot(a, xact) {
        Color(0.98, 0.45, 0.55, 1.0)
    } else if over {
        T2
    } else {
        T3
    };
    let tw = (r.w - 56.0 - if over { 38.0 } else { 12.0 }).max(40.0);
    let ty = r.y + (r.h - 43.0) / 2.0;
    g.text(&e.name, Rect::new(r.x + 56.0, ty, tw, 22.0), &f.body, T1.fade(ca), Align::Left);
    g.text(sub, Rect::new(r.x + 56.0, ty + 23.0, tw, 18.0), &f.tiny, sub_col.fade(ca), Align::Left);

    if over {
        g.fill_circle(
            xr.cx(),
            xr.cy(),
            12.0,
            Color::white((if hot(a, xact) { 0.16 } else { 0.08 }) * ca),
        );
        g.icon(
            '\u{E711}',
            xr,
            &f.icon14,
            (if hot(a, xact) { Color(0.98, 0.45, 0.55, 1.0) } else { T2 }).fade(ca),
        );
    }
}

fn page_apps(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    if a.picking {
        page_app_picker(a, ca, hits);
        return;
    }
    let g = &a.gfx;
    let f = &g.fonts;

    // Header strip: segmented borderless filter pills (All | Craft | Pinned), calm status, right actions
    let pills = [
        (AppsFilter::All, "All", 44.0),
        (AppsFilter::Craft, "Craft", 54.0),
        (AppsFilter::Pinned, "Pinned", 62.0),
    ];
    let mut px = 24.0;
    for (flt, label, pw) in pills {
        let pr = Rect::new(px, 3.0, pw, 22.0);
        let pact = Act::AppsFilterSet(flt);
        hits.push(Hit { r: pr, act: pact });
        let active = a.apps_filter == flt;
        let bg = if active {
            C_CPU.fade(0.18)
        } else if hot(a, pact) {
            HOVER
        } else {
            SURF
        };
        g.fill_rrect(pr, 11.0, bg.fade(ca));
        g.text(
            label,
            pr,
            &f.tiny,
            (if active { C_CPU } else { T2 }).fade(ca),
            Align::Center,
        );
        px += pw + 6.0;
    }

    let installed_craft = a
        .craft_states
        .iter()
        .filter(|s| matches!(s, crate::craft::State::Installed { .. }))
        .count();
    if let Some((msg, ok, t0)) = &a.craft_toast {
        if t0.elapsed().as_secs() < 6 {
            let col = if *ok { C_BAT } else { Color(0.98, 0.45, 0.55, 1.0) };
            g.text(msg, Rect::new(204.0, 2.0, 244.0, 24.0), &f.tiny, col.fade(ca), Align::Left);
        } else {
            let note = match a.apps_filter {
                AppsFilter::Craft => format!("{installed_craft}/4 installed  \u{00B7}  Rust native"),
                AppsFilter::Pinned => format!("{} pinned", a.apps.len()),
                AppsFilter::All => format!("{installed_craft}/4 Craft  \u{00B7}  {} pinned", a.apps.len()),
            };
            g.text(&note, Rect::new(204.0, 2.0, 244.0, 24.0), &f.tiny, T3.fade(ca), Align::Left);
        }
    } else {
        let note = match a.apps_filter {
            AppsFilter::Craft => format!("{installed_craft}/4 installed  \u{00B7}  Rust native"),
            AppsFilter::Pinned => format!("{} pinned", a.apps.len()),
            AppsFilter::All => format!("{installed_craft}/4 Craft  \u{00B7}  {} pinned", a.apps.len()),
        };
        g.text(&note, Rect::new(204.0, 2.0, 244.0, 24.0), &f.tiny, T3.fade(ca), Align::Left);
    }

    // Secondary header chip (ArtCraft site on Craft filter; + Add app on All/Pinned)
    if a.apps_filter == AppsFilter::Craft {
        let wb = Rect::new(EXP_W - 138.0, 2.0, 82.0, 24.0);
        hits.push(Hit { r: wb, act: Act::CraftSite });
        g.fill_rrect(wb, 12.0, (if hot(a, Act::CraftSite) { HOVER } else { SURF }).fade(ca));
        g.icon('\u{E71B}', Rect::new(wb.x + 6.0, wb.y, 22.0, wb.h), &f.icon14, C_CPU.fade(ca));
        g.text("ArtCraft", Rect::new(wb.x + 28.0, wb.y, wb.w - 34.0, wb.h), &f.tiny, T1.fade(ca), Align::Left);
    } else if a.apps.len() < 12 {
        let ab = Rect::new(EXP_W - 138.0, 2.0, 82.0, 24.0);
        hits.push(Hit { r: ab, act: Act::AppAdd });
        g.fill_rrect(ab, 12.0, (if hot(a, Act::AppAdd) { HOVER } else { SURF }).fade(ca));
        g.icon('\u{E710}', Rect::new(ab.x + 6.0, ab.y, 22.0, ab.h), &f.icon14, C_CPU.fade(ca));
        g.text("Add app", Rect::new(ab.x + 28.0, ab.y, ab.w - 34.0, ab.h), &f.tiny, T1.fade(ca), Align::Left);
    }

    // Refresh button (re-scans Craft installs & rotates during downloads)
    let rf = Rect::new(EXP_W - 50.0, 2.0, 26.0, 24.0);
    hits.push(Hit { r: rf, act: Act::CraftRefresh });
    g.fill_rrect(rf, 12.0, (if hot(a, Act::CraftRefresh) { HOVER } else { SURF }).fade(ca));
    let busy = a.craft_busy();
    let deg = if busy { a.spin_t * 360.0 } else { 0.0 };
    g.rotated(rf.cx(), rf.cy(), deg, || {
        g.icon('\u{E72C}', rf, &f.icon14, (if busy { C_CPU } else { T2 }).fade(ca))
    });

    // 2-column Controls Tile Grid (x = 24.0, w = 270.0, gap = 12.0, x = 306.0)
    let items = a.apps_items();
    let craft_only = a.apps_filter == AppsFilter::Craft;
    let (y0, th, rgap, vis_rows) = if craft_only {
        (40.0, 54.0, 10.0, 2usize)
    } else {
        (33.0, 50.0, 8.0, APP_TILE_ROWS)
    };
    let total_rows = items.len().div_ceil(2);
    let scroll_row = if craft_only {
        0
    } else {
        a.apps_scroll.min(total_rows.saturating_sub(vis_rows))
    };

    for r_idx in 0..vis_rows {
        let row = scroll_row + r_idx;
        for col in 0..2usize {
            let Some(item) = items.get(row * 2 + col) else { break };
            let rect = Rect::new(
                24.0 + col as f32 * (270.0 + 12.0),
                y0 + r_idx as f32 * (th + rgap),
                270.0,
                th,
            );
            match *item {
                AppsItem::Craft(ci) => draw_craft_tile(a, ci, rect, ca, hits),
                AppsItem::Pinned(pi) => draw_pinned_tile(a, pi, rect, ca, hits),
                AppsItem::AddApp => card_tile(
                    a,
                    ca,
                    hits,
                    rect,
                    Some(Act::AppAdd),
                    '\u{E710}',
                    C_CPU,
                    "Add app",
                    T1,
                    "Pin an installed Windows app",
                    T3,
                    None,
                    false,
                    false,
                ),
            }
        }
    }

    if total_rows > vis_rows {
        let track_h = vis_rows as f32 * (th + rgap) - rgap;
        let track = Rect::new(EXP_W - 14.0, y0, 4.0, track_h);
        g.fill_rrect(track, 2.0, Color::white(0.07 * ca));
        let thumb_h = (track_h * vis_rows as f32 / total_rows as f32).max(20.0);
        let frac = scroll_row as f32 / (total_rows - vis_rows) as f32;
        g.fill_rrect(
            Rect::new(track.x, y0 + (track_h - thumb_h) * frac.clamp(0.0, 1.0), 4.0, thumb_h),
            2.0,
            C_CPU.fade(0.75 * ca),
        );
    }
}

fn page_shelf(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    let area = Rect::new(20.0, 2.0, EXP_W - 40.0, 202.0);
    if a.shelf_hover {
        // a file is being dragged over the notch
        g.fill_rrect(area, 18.0, a.accent.fade(0.14 * ca));
        g.stroke_rrect(area, 18.0, a.accent.fade(0.9 * ca), 2.0);
        g.text("Drop to add to the shelf", area, &f.body, T1.fade(ca), Align::Center);
        return;
    }
    if a.shelf.is_empty() {
        g.stroke_rrect(area, 18.0, Color::white(0.14 * ca), 1.0);
        g.icon('\u{E896}', Rect::new(area.cx() - 20.0, 62.0, 40.0, 40.0), &f.icon22, a.accent.fade(ca));
        g.text("Drop files here to keep them handy", Rect::new(area.x, 108.0, area.w, 24.0), &f.body, T1.fade(ca), Align::Center);
        g.text("Drag them back out into any app. Click one to open it.", Rect::new(area.x, 132.0, area.w, 22.0), &f.small, T3.fade(ca), Align::Center);
        return;
    }
    let (cols, tw, th) = (6usize, 88.0, 86.0);
    let gap = (EXP_W - 56.0 - cols as f32 * tw) / (cols as f32 - 1.0);
    for (i, path) in a.shelf.iter().enumerate() {
        let (c, r) = (i % cols, i / cols);
        let rect = Rect::new(28.0 + c as f32 * (tw + gap), 4.0 + r as f32 * (th + 6.0), tw, th);
        let act = Act::ShelfItem(i);
        hits.push(Hit { r: rect, act });
        let x = Rect::new(rect.x + rect.w - 22.0, rect.y + 3.0, 19.0, 19.0);
        let xact = Act::ShelfDel(i);
        hits.push(Hit { r: x, act: xact });
        let over = hot(a, act) || hot(a, xact);
        let k = hot_k(a, act).max(hot_k(a, xact));
        g.fill_rrect(rect, 16.0, Color::white((0.05 + 0.07 * k) * ca));
        let ic = Rect::new(rect.cx() - 18.0, rect.y + 10.0, 36.0, 36.0);
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        match a.shelf_icons.get(i).and_then(|b| b.as_ref()) {
            Some(bmp) => g.draw_bitmap(bmp, ic, 0.0, ca),
            None => g.icon('\u{E8A5}', ic, &f.icon22, T2.fade(ca)),
        }
        g.text(&name, Rect::new(rect.x + 4.0, rect.y + 52.0, rect.w - 8.0, 24.0), &f.tiny, T2.fade(ca), Align::Center);
        if over {
            g.fill_circle(x.cx(), x.cy(), 8.5, Color(0.0, 0.0, 0.0, 0.75 * ca));
            g.icon('\u{E711}', x, &f.icon14, (if hot(a, xact) { Color(1.0, 0.5, 0.5, 1.0) } else { T1 }).fade(ca));
        }
    }
    let clear = Rect::new(EXP_W - 24.0 - 78.0, 190.0, 78.0, 18.0);
    hits.push(Hit { r: clear, act: Act::ShelfClear });
    g.text("Clear all", clear, &f.tiny, (if hot(a, Act::ShelfClear) { T1 } else { T3 }).fade(ca), Align::Right);
}

fn page_app_picker(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;

    // Header strip matching Controls / Work design language
    let back = Rect::new(24.0, 3.0, 62.0, 24.0);
    hits.push(Hit { r: back, act: Act::AppDone });
    g.fill_rrect(back, 12.0, (if hot(a, Act::AppDone) { HOVER } else { SURF }).fade(ca));
    g.text("\u{2039} Back", back, &f.small, T1.fade(ca), Align::Center);

    let head = if a.app_loading && a.app_list.is_some() {
        "Add an app  \u{00B7}  Updating\u{2026}"
    } else {
        "Add an app"
    };
    g.text(head, Rect::new(96.0, 3.0, 220.0, 24.0), &f.small, T2.fade(ca), Align::Left);

    let rf = Rect::new(EXP_W - 50.0, 3.0, 26.0, 24.0);
    hits.push(Hit { r: rf, act: Act::AppRefresh });
    g.fill_rrect(rf, 12.0, (if hot(a, Act::AppRefresh) { HOVER } else { SURF }).fade(ca));
    let deg = if a.app_loading { a.spin_t * 360.0 } else { 0.0 };
    g.rotated(rf.cx(), rf.cy(), deg, || {
        g.icon('\u{E72C}', rf, &f.icon14, (if a.app_loading { C_CPU } else { T2 }).fade(ca))
    });

    let sr = Rect::new(EXP_W - 236.0, 3.0, 178.0, 24.0);
    let typing = a.entry_search;
    if typing || !a.app_filter.is_empty() {
        let mut t = a.app_filter.clone();
        if typing {
            t.push('\u{2502}');
        }
        g.fill_rrect(sr, 12.0, HOVER.fade(ca));
        g.text(&t, Rect::new(sr.x + 10.0, sr.y, sr.w - 16.0, sr.h), &f.small, T1.fade(ca), Align::Left);
    } else {
        g.fill_rrect(sr, 12.0, (if hot(a, Act::AppSearch) { HOVER } else { SURF }).fade(ca));
        g.icon('\u{E721}', Rect::new(sr.x + 6.0, sr.y, 22.0, sr.h), &f.icon14, T3.fade(ca));
        g.text("Search apps", Rect::new(sr.x + 29.0, sr.y, sr.w - 34.0, sr.h), &f.small, T3.fade(ca), Align::Left);
    }
    hits.push(Hit { r: sr, act: Act::AppSearch });

    let Some(_) = &a.app_list else {
        if a.app_load_t0.is_some_and(|t| t.elapsed().as_secs() >= 15) {
            card_tile(
                a,
                ca,
                hits,
                Rect::new(24.0, 66.0, 552.0, 56.0),
                Some(Act::AppRetry),
                '\u{E72C}',
                C_MEM,
                "Windows is taking longer than usual",
                T1,
                "Click to scan your Start menu apps again",
                T3,
                None,
                false,
                false,
            );
        } else {
            spinner(g, EXP_W / 2.0, 96.0, 15.0, a.spin_t, C_CPU, ca);
            g.text("Finding your apps\u{2026}", Rect::new(0.0, 124.0, EXP_W, 22.0), &f.small, T2.fade(ca), Align::Center);
        }
        return;
    };

    let view = a.app_view();
    if view.is_empty() {
        let (l1, l2) = if a.app_filter.is_empty() {
            ("No apps found", "Click refresh to scan installed Windows apps")
        } else {
            ("No apps match that search", "Press Esc in search to clear the filter")
        };
        card_tile(
            a,
            ca,
            hits,
            Rect::new(24.0, 66.0, 552.0, 56.0),
            Some(Act::AppRefresh),
            '\u{E721}',
            C_CPU,
            l1,
            T1,
            l2,
            T3,
            None,
            false,
            false,
        );
        return;
    }

    // 4 compact borderless matte rows (37px tall, r = 14.0, 25x25 circular badge)
    for row in 0..PICK_ROWS {
        let idx = a.app_scroll + row;
        let Some((name, id)) = view.get(idx) else { break };
        let r = Rect::new(24.0, 34.0 + row as f32 * 41.0, EXP_W - 48.0, 37.0);
        let target = format!("shell:AppsFolder\\{id}");
        let pinned = a.apps.iter().any(|p| p.target == target);
        let act = Act::AppPick(idx);
        if !pinned {
            hits.push(Hit { r, act });
        }
        let k = if pinned { 0.0 } else { hot_k(a, act) };
        g.fill_rrect(r, 14.0, Color::white((0.09 + 0.08 * k) * ca));

        let hue = name_color(name);
        let ic = Rect::new(r.x + 8.5, r.y + (r.h - 25.0) / 2.0, 25.0, 25.0);
        g.fill_circle(ic.cx(), ic.cy(), 12.5, hue.fade(0.16 * ca));
        match a.picker_icons.iter().find(|(k, _)| k == id).and_then(|(_, b)| b.as_ref()) {
            Some(bmp) => g.draw_bitmap(bmp, Rect::new(ic.cx() - 9.0, ic.cy() - 9.0, 18.0, 18.0), 3.0, ca),
            None => {
                let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                g.text(&letter, ic, &f.tiny, hue.fade(ca), Align::Center);
            }
        }

        let on = hot(a, act) && !pinned;
        let pill_w = if pinned { 70.0 } else if on { 84.0 } else { 0.0 };
        if pill_w > 0.0 {
            let pr = Rect::new(r.x + r.w - 10.0 - pill_w, r.y + 7.5, pill_w, 22.0);
            let pcol = if pinned { C_BAT } else { C_CPU };
            g.fill_rrect(pr, 11.0, pcol.fade(0.16 * ca));
            g.text(
                if pinned { "Pinned" } else { "Pin app" },
                pr,
                &f.tiny,
                pcol.fade(ca),
                Align::Center,
            );
        }

        let tx = r.x + 42.0;
        let tw = (r.w - 54.0 - pill_w).max(60.0);
        g.text(name, Rect::new(tx, r.y + 2.0, tw, 19.0), &f.small, (if pinned { T2 } else { T1 }).fade(ca), Align::Left);
        let sub = if pinned { "Already pinned to Apps" } else { "Installed Windows app  \u{00B7}  Click to pin" };
        g.text(sub, Rect::new(tx, r.y + 20.0, tw, 15.0), &f.tiny, T3.fade(ca), Align::Left);
    }

    if view.len() > PICK_ROWS {
        let (top, h) = (34.0, PICK_ROWS as f32 * 41.0 - 4.0);
        let track = Rect::new(EXP_W - 14.0, top, 4.0, h);
        g.fill_rrect(track, 2.0, Color::white(0.07 * ca));
        let thumb_h = (h * PICK_ROWS as f32 / view.len() as f32).max(18.0);
        let frac = a.app_scroll as f32 / (view.len() - PICK_ROWS) as f32;
        g.fill_rrect(
            Rect::new(track.x, top + (h - thumb_h) * frac.clamp(0.0, 1.0), 4.0, thumb_h),
            2.0,
            C_CPU.fade(0.75 * ca),
        );
    }
}

// ----------------------------------------------------------------- tasks ----

fn page_tasks(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;

    // ---- cards
    for (r, c) in [(Rect::new(14.0, 2.0, 208.0, 198.0), C_FOC), (Rect::new(230.0, 2.0, 190.0, 198.0), C_BAT), (Rect::new(428.0, 2.0, 158.0, 198.0), C_UP)] {
        g.fill_rrect(r, 20.0, Color::white(0.05 * ca));
        g.fill_rrect(r, 20.0, c.fade(0.07 * ca));
    }

    // ---- focus timer
    let title = if a.break_mode {
        "Break".to_string()
    } else if let Some(t) = &a.timer_task {
        let short: String = t.chars().take(20).collect();
        format!("Focus \u{00B7} {short}")
    } else {
        "Focus timer".to_string()
    };
    g.text(
        &title,
        Rect::new(28.0, 12.0, 185.0, 22.0),
        &f.small,
        (if a.break_mode { a.accent } else { C_FOC }).fade(ca),
        Align::Left,
    );
    g.text(
        &format!("Today: {} min focus \u{00B7} {} done", a.focus_today, a.done_today),
        Rect::new(28.0, 176.0, 190.0, 16.0),
        &f.tiny,
        T3.fade(ca),
        Align::Left,
    );
    let left = a.timer_remaining_ms();
    let done = !a.timer_running() && left == 0;
    let face = match (&a.entering, a.entry_timer) {
        (Some(buf), true) => format!("{}\u{2502} min", String::from_utf16_lossy(buf)),
        _ => fmt_time(left as f64 / 1000.0),
    };
    g.text(
        &face,
        Rect::new(28.0, 34.0, 175.0, 52.0),
        &f.big,
        (if done && !a.entry_timer { C_FOC } else { T1 }).fade(ca),
        Align::Left,
    );
    if a.entry_timer {
        g.text(
            "Enter to start \u{00B7} Esc to cancel",
            Rect::new(28.0, 76.0, 190.0, 16.0),
            &f.tiny,
            T3.fade(ca),
            Align::Left,
        );
    }
    if done {
        g.text(
            "Time's up",
            Rect::new(110.0, 12.0, 90.0, 22.0),
            &f.small,
            C_FOC.fade(ca),
            Align::Right,
        );
    }
    for (i, mins) in [5u32, 15, 25, 50].iter().enumerate() {
        let r = Rect::new(28.0 + i as f32 * 38.0, 94.0, 34.0, 26.0);
        let act = Act::TimerPreset(*mins);
        hits.push(Hit { r, act });
        let sel = a.timer_total_s == mins * 60;
        let bg = if hot(a, act) {
            HOVER
        } else if sel {
            C_FOC.fade(0.28)
        } else {
            SURF
        };
        g.fill_rrect(r, 13.0, bg.fade(ca));
        g.text(
            &format!("{}m", mins),
            r,
            &f.small,
            (if sel { C_FOC } else { T2 }).fade(ca),
            Align::Center,
        );
    }
    // custom length: type minutes
    let cr = Rect::new(28.0 + 4.0 * 38.0, 94.0, 34.0, 26.0);
    hits.push(Hit { r: cr, act: Act::TimerCustom });
    g.fill_rrect(cr, 13.0, (if hot(a, Act::TimerCustom) || a.entry_timer { HOVER } else { SURF }).fade(ca));
    g.icon('\u{E70F}', cr, &f.icon14, T2.fade(ca));
    let sb = Rect::new(28.0, 130.0, 108.0, 36.0);
    hits.push(Hit {
        r: sb,
        act: Act::TimerToggle,
    });
    g.fill_rrect(
        sb,
        18.0,
        C_FOC.fade((if hot(a, Act::TimerToggle) { 1.0 } else { 0.9 }) * ca),
    );
    g.text(
        if a.timer_running() { "Pause" } else { "Start" },
        sb,
        &f.body,
        Color(0.0, 0.0, 0.0, ca),
        Align::Center,
    );
    let rb = Rect::new(144.0, 130.0, 36.0, 36.0);
    hits.push(Hit {
        r: rb,
        act: Act::TimerReset,
    });
    g.fill_rrect(
        rb,
        18.0,
        (if hot(a, Act::TimerReset) { HOVER } else { SURF }).fade(ca),
    );
    g.icon('\u{E72C}', rb, &f.icon14, T1.fade(ca));

    // ---- to-do
    let tx = 240.0;
    let tw = 170.0;
    g.text(
        "To-do",
        Rect::new(tx, 12.0, 100.0, 22.0),
        &f.small,
        C_BAT.fade(ca),
        Align::Left,
    );
    for (i, t) in a.todos.iter().enumerate() {
        let y = 38.0 + i as f32 * 28.0;
        let row = Rect::new(tx, y, tw, 26.0);
        let cb = Rect::new(tx, y, 26.0, 26.0);
        let del = Rect::new(tx + tw - 24.0, y, 24.0, 26.0);
        let fb = Rect::new(tx + tw - 50.0, y, 24.0, 26.0);
        if !t.done {
            hits.push(Hit { r: fb, act: Act::TodoFocus(i) });
        }
        hits.push(Hit {
            r: cb,
            act: Act::TodoToggle(i),
        });
        hits.push(Hit {
            r: del,
            act: Act::TodoDel(i),
        });
        let over = hot(a, Act::TodoToggle(i)) || hot(a, Act::TodoDel(i)) || hot(a, Act::TodoFocus(i));
        if over {
            g.fill_rrect(row, 9.0, Color::white(0.07 * ca));
        }
        let glyph = if t.done { '\u{E73A}' } else { '\u{E739}' };
        let gc = if t.done { C_BAT } else { T2 };
        g.icon(glyph, cb, &f.icon14, gc.fade(ca));
        g.text(
            &t.text,
            Rect::new(tx + 30.0, y, tw - 30.0 - 26.0, 26.0),
            &f.small,
            (if t.done { T3 } else { T1 }).fade(ca),
            Align::Left,
        );
        if over {
            g.fill_rrect(Rect::new(tx + tw - 52.0, y + 1.0, 52.0, 24.0), 8.0, Color(0.117, 0.117, 0.117, 1.0).fade(ca));
        }
        if !t.done {
            g.icon(
                '\u{E768}',
                fb,
                &f.icon14,
                (if hot(a, Act::TodoFocus(i)) { a.accent } else { T3 }).fade(ca * if over { 1.0 } else { 0.0 }),
            );
        }
        g.icon(
            '\u{E711}',
            del,
            &f.icon14,
            (if hot(a, Act::TodoDel(i)) { T1 } else { T3 }).fade(ca * if over { 1.0 } else { 0.0 }),
        );
    }
    let y = 38.0 + a.todos.len() as f32 * 28.0;
    if let (Some(buf), false) = (&a.entering, a.entry_timer) {
        let r = Rect::new(tx, y, tw, 26.0);
        g.fill_rrect(r, 9.0, Color::white(0.14 * ca));
        let mut s = String::from_utf16_lossy(buf);
        s.push('\u{2502}'); // caret
        g.text(
            &s,
            Rect::new(r.x + 10.0, r.y, r.w - 16.0, r.h),
            &f.small,
            T1.fade(ca),
            Align::Left,
        );
        g.text(
            "Enter to add \u{00B7} Esc to cancel",
            Rect::new(tx, y + 28.0, tw, 18.0),
            &f.tiny,
            T3.fade(ca),
            Align::Left,
        );
    } else if a.todos.len() < MAX_TODO {
        let r = Rect::new(tx, y, tw, 26.0);
        hits.push(Hit {
            r,
            act: Act::TodoAdd,
        });
        if hot(a, Act::TodoAdd) {
            g.fill_rrect(r, 9.0, Color::white(0.07 * ca));
        }
        g.icon(
            '\u{E710}',
            Rect::new(tx, y, 26.0, 26.0),
            &f.icon14,
            C_BAT.fade(ca),
        );
        g.text(
            "Add task",
            Rect::new(tx + 30.0, y, 140.0, 26.0),
            &f.small,
            T3.fade(ca),
            Align::Left,
        );
    }

    // ---- calendar
    draw_calendar(a, ca);
}

// ------------------------------------------------------------------ work ----

/// Semantic Segoe Fluent / MDL2 icon glyph per Sprint Backlog status.
fn work_status_glyph(status: &str) -> char {
    match status.trim() {
        "Completed" => '\u{E73E}',
        "Working" => '\u{E916}',
        "Overdue" | "For Hotfix" => '\u{E783}',
        "Pending Review" | "For Release Tag" => '\u{E734}',
        "On Hold" => '\u{E769}',
        "Closed" | "Cancelled" => '\u{E711}',
        _ => '\u{E8A5}',
    }
}

fn page_work(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    if let Some(item) = a.work_current() {
        page_work_detail(a, item, ca, hits);
        return;
    }
    page_work_list(a, ca, hits);
}

fn page_work_list(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    let view = a.work_view();

    // Header row: calm title + count, borderless All/Mine chip, search pill, refresh circle
    let head = if a.work_items.is_empty() {
        "Sprint Backlogs".to_string()
    } else {
        format!("Sprint Backlogs  \u{00B7}  {}", view.len())
    };
    g.text(&head, Rect::new(26.0, 3.0, 168.0, 24.0), &f.small, T2.fade(ca), Align::Left);

    if !a.work_items.is_empty() {
        let scope = Rect::new(192.0, 4.0, 54.0, 22.0);
        hits.push(Hit { r: scope, act: Act::WorkScopeToggle });
        let on = a.work_mine_only;
        let bg = if on {
            C_CPU.fade(0.18)
        } else if hot(a, Act::WorkScopeToggle) {
            HOVER
        } else {
            SURF
        };
        g.fill_rrect(scope, 11.0, bg.fade(ca));
        g.text(
            if on { "Mine" } else { "All" },
            scope,
            &f.tiny,
            (if on { C_CPU } else { T2 }).fade(ca),
            Align::Center,
        );
    }

    // Refresh button (borderless SURF -> HOVER)
    let rf = Rect::new(EXP_W - 50.0, 3.0, 26.0, 24.0);
    hits.push(Hit { r: rf, act: Act::WorkRefresh });
    g.fill_rrect(rf, 12.0, (if hot(a, Act::WorkRefresh) { HOVER } else { SURF }).fade(ca));
    let deg = if a.work_loading { a.spin_t * 360.0 } else { 0.0 };
    g.rotated(rf.cx(), rf.cy(), deg, || {
        g.icon('\u{E72C}', rf, &f.icon14, (if a.work_loading { C_CPU } else { T2 }).fade(ca))
    });

    // Search pill (borderless SURF -> HOVER)
    let sr = Rect::new(EXP_W - 212.0, 3.0, 154.0, 24.0);
    let typing = a.entry_search && a.work_drop.is_none();
    if typing || !a.work_filter.is_empty() {
        let mut t = a.work_filter.clone();
        if typing {
            t.push('\u{2502}');
        }
        g.fill_rrect(sr, 12.0, HOVER.fade(ca));
        g.text(&t, Rect::new(sr.x + 10.0, sr.y, sr.w - 30.0, sr.h), &f.small, T1.fade(ca), Align::Left);
        if !typing {
            let x = Rect::new(sr.x + sr.w - 24.0, sr.y, 24.0, sr.h);
            hits.push(Hit { r: sr, act: Act::WorkSearch });
            hits.push(Hit { r: x, act: Act::WorkSearchClear });
            g.icon('\u{E711}', x, &f.icon14, (if hot(a, Act::WorkSearchClear) { T1 } else { T2 }).fade(ca));
        }
    } else {
        hits.push(Hit { r: sr, act: Act::WorkSearch });
        g.fill_rrect(sr, 12.0, (if hot(a, Act::WorkSearch) { HOVER } else { SURF }).fade(ca));
        g.icon('\u{E721}', Rect::new(sr.x + 6.0, sr.y, 22.0, sr.h), &f.icon14, T3.fade(ca));
        g.text("Search tasks", Rect::new(sr.x + 29.0, sr.y, sr.w - 34.0, sr.h), &f.small, T3.fade(ca), Align::Left);
    }

    // Loading / error / empty states
    if a.work_loading && a.work_items.is_empty() {
        spinner(g, EXP_W / 2.0, 96.0, 15.0, a.spin_t, C_CPU, ca);
        g.text("Syncing Sprint Backlogs\u{2026}", Rect::new(0.0, 124.0, EXP_W, 22.0), &f.small, T2.fade(ca), Align::Center);
        return;
    }

    if let Some(err) = &a.work_error {
        if a.work_items.is_empty() {
            // Controls-style status banner + 2-column action tiles
            card_tile(
                a,
                ca,
                hits,
                Rect::new(24.0, 38.0, 552.0, 52.0),
                None,
                '\u{E783}',
                Color(0.98, 0.45, 0.55, 1.0),
                "ERPNext connection required",
                T1,
                err,
                T3,
                None,
                false,
                false,
            );
            card_tile(
                a,
                ca,
                hits,
                Rect::new(24.0, 98.0, 270.0, 54.0),
                Some(Act::WorkOpenConfig),
                '\u{E713}',
                C_MEM,
                "Configure ERPNext",
                T1,
                "Edit erp_url & erp_token in config.ini",
                T3,
                None,
                false,
                false,
            );
            card_tile(
                a,
                ca,
                hits,
                Rect::new(306.0, 98.0, 270.0, 54.0),
                Some(Act::WorkRefresh),
                '\u{E72C}',
                C_CPU,
                "Retry sync",
                T1,
                "Reconnect to Sprint Backlogs",
                T3,
                None,
                false,
                false,
            );
            return;
        }
    }

    if view.is_empty() {
        let (l1, l2) = if !a.work_filter.is_empty() {
            ("No Sprint Backlogs match your filter", "Press Esc in search or click \u{00D7} to clear")
        } else if a.work_mine_only {
            ("No Sprint Backlogs assigned to you", "Click 'Mine' above to view all tasks")
        } else {
            ("No Sprint Backlogs found", "Click refresh to check ERPNext")
        };
        card_tile(
            a,
            ca,
            hits,
            Rect::new(24.0, 68.0, 552.0, 56.0),
            Some(Act::WorkRefresh),
            '\u{E8A5}',
            C_CPU,
            l1,
            T1,
            l2,
            T3,
            None,
            false,
            false,
        );
        return;
    }

    // 4 borderless Controls-style rows: tinted circular icon badge + 2-line hierarchy + soft status pill
    for row in 0..WORK_ROWS {
        let idx = a.work_scroll + row;
        let Some(item) = view.get(idx) else { break };
        let r = Rect::new(24.0, 34.0 + row as f32 * 41.0, EXP_W - 48.0, 37.0);
        let act = Act::WorkSelect(idx);
        hits.push(Hit { r, act });
        let k = hot_k(a, act);
        let scol = crate::work::status_color(&item.status);

        g.fill_rrect(r, 14.0, Color::white((0.09 + 0.08 * k) * ca));

        // Soft tinted circular status icon badge on the left
        let ic = Rect::new(r.x + 10.0, r.y + (r.h - 26.0) / 2.0, 26.0, 26.0);
        g.fill_circle(ic.cx(), ic.cy(), 13.0, scol.fade(0.16 * ca));
        g.icon(work_status_glyph(&item.status), ic, &f.icon14, scol.fade(ca));

        // Right-aligned borderless status pill + optional comment badge
        let cmt_w = if item.comments > 0 { 38.0 } else { 0.0 };
        if item.comments > 0 {
            let cr = Rect::new(r.x + r.w - 42.0, r.y + 7.5, 32.0, 22.0);
            g.fill_rrect(cr, 11.0, C_UP.fade(0.15 * ca));
            g.text(&format!("{}", item.comments), cr, &f.tiny, C_UP.fade(ca), Align::Center);
        }
        let pill_w = (item.status.chars().count() as f32 * 6.0 + 18.0).clamp(64.0, 112.0);
        let pr = Rect::new(r.x + r.w - 10.0 - cmt_w - pill_w, r.y + 7.5, pill_w, 22.0);
        g.fill_rrect(pr, 11.0, scol.fade(0.15 * ca));
        g.text(&item.status, pr, &f.tiny, scol.fade(ca), Align::Center);

        // Two-line stacked title + metadata subtitle beside the circular icon badge
        let tx = r.x + 46.0;
        let tw = (pr.x - tx - 10.0).max(60.0);
        g.text(&item.subject, Rect::new(tx, r.y + 2.0, tw, 19.0), &f.small, T1.fade(ca), Align::Left);

        let mut sub = item.name.clone();
        if !item.current_assignee.is_empty() {
            sub.push_str("  \u{00B7}  ");
            sub.push_str(&a.work_user_label(&item.current_assignee));
        } else if !item.priority.is_empty() {
            sub.push_str("  \u{00B7}  ");
            sub.push_str(&item.priority);
        }
        if !item.sprint.is_empty() {
            sub.push_str("  \u{00B7}  ");
            sub.push_str(&item.sprint);
        }
        g.text(&sub, Rect::new(tx, r.y + 20.0, tw, 15.0), &f.tiny, T3.fade(ca), Align::Left);
    }

    if view.len() > WORK_ROWS {
        let (top, h) = (34.0, WORK_ROWS as f32 * 41.0 - 4.0);
        let track = Rect::new(EXP_W - 16.0, top, 4.0, h);
        g.fill_rrect(track, 2.0, Color::white(0.07 * ca));
        let thumb_h = (h * WORK_ROWS as f32 / view.len() as f32).max(18.0);
        let frac = a.work_scroll as f32 / (view.len() - WORK_ROWS) as f32;
        g.fill_rrect(
            Rect::new(track.x, top + (h - thumb_h) * frac.clamp(0.0, 1.0), 4.0, thumb_h),
            2.0,
            C_CPU.fade(0.8 * ca),
        );
    }
}

fn page_work_detail(a: &App, item: &crate::work::BacklogItem, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    let scol = crate::work::status_color(&item.status);

    // ---- Top navigation strip: borderless Back chip + Save/Toast status + Refresh circle
    let back = Rect::new(24.0, 0.0, 64.0, 22.0);
    hits.push(Hit { r: back, act: Act::WorkBack });
    g.fill_rrect(back, 11.0, (if hot(a, Act::WorkBack) { HOVER } else { SURF }).fade(ca));
    g.icon('\u{E76B}', Rect::new(back.x + 6.0, back.y, 16.0, back.h), &f.icon14, T2.fade(ca));
    g.text("Back", Rect::new(back.x + 22.0, back.y, 36.0, back.h), &f.tiny, T1.fade(ca), Align::Left);

    let rf = Rect::new(EXP_W - 48.0, 0.0, 24.0, 22.0);
    hits.push(Hit { r: rf, act: Act::WorkRefresh });
    g.fill_rrect(rf, 11.0, (if hot(a, Act::WorkRefresh) { HOVER } else { SURF }).fade(ca));
    let deg = if a.work_loading { a.spin_t * 360.0 } else { 0.0 };
    g.rotated(rf.cx(), rf.cy(), deg, || {
        g.icon('\u{E72C}', rf, &f.icon14, (if hot(a, Act::WorkRefresh) { T1 } else { T2 }).fade(ca))
    });

    if a.work_saving {
        spinner(g, EXP_W - 118.0, 11.0, 5.5, a.spin_t, C_CPU, ca);
        g.text("Saving\u{2026}", Rect::new(EXP_W - 106.0, 0.0, 54.0, 22.0), &f.tiny, C_CPU.fade(ca), Align::Left);
    } else if let Some((msg, ok, t0)) = &a.work_toast {
        if t0.elapsed().as_secs_f32() < 4.0 {
            let tc = if *ok { C_BAT } else { Color(0.98, 0.45, 0.55, 1.0) };
            let label = if *ok { format!("\u{2713} {msg}") } else { ellipsize(msg, 28) };
            g.text(&label, Rect::new(EXP_W - 240.0, 0.0, 184.0, 22.0), &f.tiny, tc.fade(ca), Align::Right);
        }
    }

    // ---- Row 1: Full-width Backlog Overview Tile (24..576, matches 2-column grid edges)
    let mut meta = item.name.clone();
    if !item.priority.is_empty() {
        meta.push_str("  \u{00B7}  ");
        meta.push_str(&item.priority);
    }
    if !item.sprint.is_empty() {
        meta.push_str("  \u{00B7}  ");
        meta.push_str(&item.sprint);
    }
    card_tile(
        a,
        ca,
        hits,
        Rect::new(24.0, 28.0, 552.0, 50.0),
        None,
        '\u{E8A5}',
        C_MEM,
        &item.subject,
        T1,
        &meta,
        T3,
        None,
        false,
        false,
    );

    // ---- Row 2: 2-Column Controls Grid (Assignee & Status, 270x50 each, 12px gap)
    let asg_box = Rect::new(24.0, 86.0, 270.0, 50.0);
    let asg_open = a.work_drop == Some(WorkDrop::Assignee);
    let asg_label = a.work_user_label(&item.current_assignee);
    let asg_tint = if item.current_assignee.is_empty() { C_CPU } else { name_color(&asg_label) };
    let (asg_title, asg_tc) = if asg_open {
        if a.work_drop_query.is_empty() {
            ("Search user\u{2026}\u{2502}".to_string(), T2)
        } else {
            (format!("{}\u{2502}", a.work_drop_query), T1)
        }
    } else if item.current_assignee.is_empty() {
        ("Unassigned".to_string(), T2)
    } else {
        (asg_label, T1)
    };
    let (asg_sub, asg_sc) = if asg_open {
        ("Type to search \u{00B7} Esc to close", asg_tint)
    } else {
        ("Current assignee", T3)
    };
    card_tile(
        a,
        ca,
        hits,
        asg_box,
        Some(Act::WorkAssignee),
        if asg_open { '\u{E721}' } else { '\u{E77B}' },
        asg_tint,
        &asg_title,
        asg_tc,
        asg_sub,
        asg_sc,
        Some(('\u{E70D}', if asg_open { asg_tint } else { T3 })),
        asg_open,
        false,
    );

    let st_box = Rect::new(306.0, 86.0, 270.0, 50.0);
    let st_open = a.work_drop == Some(WorkDrop::Status);
    let (st_title, st_tc) = if st_open {
        if a.work_drop_query.is_empty() {
            ("Filter status\u{2026}\u{2502}".to_string(), T2)
        } else {
            (format!("{}\u{2502}", a.work_drop_query), T1)
        }
    } else {
        (item.status.clone(), T1)
    };
    let (st_sub, st_sc) = if st_open {
        ("Type to filter \u{00B7} Esc to close", scol)
    } else {
        ("Task status", T3)
    };
    card_tile(
        a,
        ca,
        hits,
        st_box,
        Some(Act::WorkStatus),
        if st_open { '\u{E721}' } else { work_status_glyph(&item.status) },
        scol,
        &st_title,
        st_tc,
        st_sub,
        st_sc,
        Some(('\u{E70D}', if st_open { scol } else { T3 })),
        st_open,
        false,
    );

    // ---- Row 3: 2-Column Action Tiles (hidden directly beneath an open dropdown so short lists never clip)
    if !asg_open {
        let open_sub = format!("View {} in browser", item.name);
        card_tile(
            a,
            ca,
            hits,
            Rect::new(24.0, 144.0, 270.0, 50.0),
            Some(Act::WorkOpenBrowser),
            '\u{E8A7}',
            C_CPU,
            "Open in ERPNext",
            T1,
            &open_sub,
            T3,
            None,
            false,
            false,
        );
    }

    if !st_open {
        let cmt_title = match item.comments {
            0 => "No comments".to_string(),
            1 => "1 comment".to_string(),
            n => format!("{n} comments"),
        };
        let cmt_sub = if item.comments == 0 {
            "Open activity & start discussion"
        } else {
            "Open discussion & activity"
        };
        card_tile(
            a,
            ca,
            hits,
            Rect::new(306.0, 144.0, 270.0, 50.0),
            Some(Act::WorkOpenComments),
            '\u{E8BD}',
            C_UP,
            &cmt_title,
            T1,
            cmt_sub,
            T3,
            None,
            false,
            false,
        );
    }

    // ---- Searchable Dropdown Overlay (drawn last so it floats cleanly above Row 3)
    if let Some(kind) = a.work_drop {
        hits.push(Hit {
            r: Rect::new(0.0, 0.0, EXP_W, EXP_H - CONTENT_Y),
            act: Act::WorkCloseDrop,
        });
        hits.push(Hit { r: asg_box, act: Act::WorkAssignee });
        hits.push(Hit { r: st_box, act: Act::WorkStatus });

        let opts = a.work_drop_options_for(kind);
        let row_h = 21.5;
        let vis_rows = opts.len().clamp(1, WORK_DROP_ROWS);
        let drop_h = 6.0 + vis_rows as f32 * row_h;
        let drop_r = match kind {
            WorkDrop::Assignee => Rect::new(24.0, 136.5, 270.0, drop_h),
            WorkDrop::Status => Rect::new(306.0, 136.5, 270.0, drop_h),
        };
        // Borderless matte dark surface
        g.fill_rrect(drop_r, 16.0, BLACK.fade(ca));
        g.fill_rrect(drop_r, 16.0, Color::white(0.11 * ca));
        let accent_col = match kind {
            WorkDrop::Assignee => asg_tint,
            WorkDrop::Status => scol,
        };

        if opts.is_empty() {
            if kind == WorkDrop::Assignee && a.work_searching {
                spinner(g, drop_r.cx() - 48.0, drop_r.cy(), 5.0, a.spin_t, C_CPU, ca);
                g.text("Searching users\u{2026}", drop_r, &f.small, T2.fade(ca), Align::Center);
            } else {
                g.text("No matches", drop_r, &f.small, T3.fade(ca), Align::Center);
            }
        } else {
            for row in 0..WORK_DROP_ROWS {
                let idx = a.work_drop_scroll + row;
                let Some((val, sub)) = opts.get(idx) else { break };
                let rr = Rect::new(drop_r.x + 4.0, drop_r.y + 2.5 + row as f32 * row_h, drop_r.w - 12.0, row_h - 1.0);
                let oact = Act::WorkPickOption(idx);
                hits.push(Hit { r: rr, act: oact });
                let is_sel = idx == a.work_drop_sel;
                let is_hot = hot(a, oact);
                if is_hot || is_sel {
                    g.fill_rrect(rr, 9.0, Color::white((if is_hot { 0.16 } else { 0.10 }) * ca));
                }
                match kind {
                    WorkDrop::Assignee => {
                        let active = item.current_assignee.eq_ignore_ascii_case(val);
                        let title = if val.is_empty() {
                            "Unassigned"
                        } else if !sub.is_empty() {
                            sub.as_str()
                        } else {
                            val.as_str()
                        };
                        let dc = if val.is_empty() { T3 } else { name_color(title) };
                        g.fill_circle(rr.x + 11.0, rr.cy(), 4.5, dc.fade(ca));
                        g.text(
                            title,
                            Rect::new(rr.x + 22.0, rr.y, rr.w - 46.0, rr.h),
                            &f.small,
                            (if val.is_empty() { T2 } else { T1 }).fade(ca),
                            Align::Left,
                        );
                        if active {
                            g.icon('\u{E73E}', Rect::new(rr.x + rr.w - 22.0, rr.y, 18.0, rr.h), &f.icon14, accent_col.fade(ca));
                        }
                    }
                    WorkDrop::Status => {
                        let active = item.status.eq_ignore_ascii_case(val);
                        let oc = crate::work::status_color(val);
                        g.fill_circle(rr.x + 11.0, rr.cy(), 4.5, oc.fade(ca));
                        g.text(val, Rect::new(rr.x + 22.0, rr.y, rr.w - 46.0, rr.h), &f.small, T1.fade(ca), Align::Left);
                        if active {
                            g.icon('\u{E73E}', Rect::new(rr.x + rr.w - 22.0, rr.y, 18.0, rr.h), &f.icon14, oc.fade(ca));
                        }
                    }
                }
            }
            if opts.len() > WORK_DROP_ROWS {
                let (top, h) = (drop_r.y + 6.0, drop_r.h - 12.0);
                let track = Rect::new(drop_r.x + drop_r.w - 6.0, top, 3.0, h);
                g.fill_rrect(track, 1.5, Color::white(0.08 * ca));
                let th = (h * WORK_DROP_ROWS as f32 / opts.len() as f32).max(14.0);
                let frac = a.work_drop_scroll as f32 / (opts.len() - WORK_DROP_ROWS) as f32;
                g.fill_rrect(
                    Rect::new(track.x, top + (h - th) * frac.clamp(0.0, 1.0), 3.0, th),
                    1.5,
                    accent_col.fade(0.8 * ca),
                );
            }
            if kind == WorkDrop::Assignee && a.work_searching {
                spinner(g, drop_r.x + drop_r.w - 14.0, drop_r.y + 10.0, 4.0, a.spin_t, C_CPU, ca);
            }
        }
    }
}

// ------------------------------------------------------------------ chat ----

fn page_chat(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    if !a.chat_signed_in {
        page_chat_signed_out(a, ca, hits);
        return;
    }
    page_chat_room(a, ca, hits);
}

fn page_chat_signed_out(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;

    // Header row: calm title + toast
    g.text(
        "ChatGPT & Codex",
        Rect::new(26.0, 3.0, 180.0, 24.0),
        &f.small,
        T2.fade(ca),
        Align::Left,
    );

    if let Some((msg, ok, t0)) = &a.chat_toast {
        if t0.elapsed().as_secs_f32() < 4.0 {
            let tc = if *ok { C_BAT } else { Color(0.98, 0.45, 0.55, 1.0) };
            g.text(
                &ellipsize(msg, 36),
                Rect::new(EXP_W - 250.0, 3.0, 226.0, 24.0),
                &f.tiny,
                tc.fade(ca),
                Align::Right,
            );
        }
    }

    // Hero action tile for signing in with ChatGPT only
    let sign_r = Rect::new(24.0, 48.0, 552.0, 56.0);
    if a.chat_signing_in {
        card_tile(
            a,
            ca,
            hits,
            sign_r,
            Some(Act::ChatCancelSignIn),
            '\u{E711}',
            Color(0.98, 0.45, 0.55, 1.0),
            "Waiting for ChatGPT sign-in in browser\u{2026}",
            T1,
            "Click to cancel \u{00B7} Browser OAuth PKCE callback on localhost:1455",
            C_MEM,
            None,
            true,
            false,
        );
        spinner(g, sign_r.x + sign_r.w - 24.0, sign_r.cy(), 6.5, a.spin_t, C_MEM, ca);
    } else {
        card_tile(
            a,
            ca,
            hits,
            sign_r,
            Some(Act::ChatSignIn),
            '\u{E950}',
            C_BAT,
            "Sign in with ChatGPT",
            T1,
            "Bring your own ChatGPT or Codex account to chat inside Vey",
            T3,
            None,
            false,
            false,
        );
    }

    if let Some(err) = &a.chat_error {
        card_tile(
            a,
            ca,
            hits,
            Rect::new(24.0, 116.0, 552.0, 48.0),
            None,
            '\u{E783}',
            Color(0.98, 0.45, 0.55, 1.0),
            "Connection notice",
            T1,
            err,
            T3,
            None,
            false,
            false,
        );
    }
}

fn page_chat_room(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;

    // ---- Top Header Strip (x = 24..576, y = 2.0, h = 22.0)
    let has_toast = a
        .chat_toast
        .as_ref()
        .is_some_and(|(_, _, t0)| t0.elapsed().as_secs_f32() < 3.5);
    let acct_w = if has_toast { 206.0 } else { 298.0 };
    let acct_r = Rect::new(24.0, 2.0, acct_w, 22.0);
    g.fill_rrect(acct_r, 11.0, SURF.fade(ca));
    g.fill_circle(acct_r.x + 11.0, acct_r.cy(), 4.0, C_BAT.fade(ca));
    let acct_text = if a.chat_account.is_empty() {
        "ChatGPT Connected".to_string()
    } else {
        ellipsize(&a.chat_account, if has_toast { 27 } else { 42 })
    };
    g.text(
        &acct_text,
        Rect::new(acct_r.x + 21.0, acct_r.y, acct_r.w - 27.0, acct_r.h),
        &f.tiny,
        T2.fade(ca),
        Align::Left,
    );

    if let Some((msg, ok, t0)) = &a.chat_toast {
        if t0.elapsed().as_secs_f32() < 3.5 {
            let tc = if *ok { C_BAT } else { Color(0.98, 0.45, 0.55, 1.0) };
            g.text(
                &format!("\u{2713} {msg}"),
                Rect::new(236.0, 2.0, 94.0, 22.0),
                &f.tiny,
                tc.fade(ca),
                Align::Left,
            );
        }
    }

    // Model selector chip
    let mdl_r = Rect::new(EXP_W - 266.0, 2.0, 148.0, 22.0);
    hits.push(Hit {
        r: mdl_r,
        act: Act::ChatModelDrop,
    });
    let mdl_bg = if a.chat_model_open || hot(a, Act::ChatModelDrop) {
        HOVER
    } else {
        SURF
    };
    g.fill_rrect(mdl_r, 11.0, mdl_bg.fade(ca));
    if a.chat_model_open {
        g.fill_rrect(mdl_r, 11.0, C_UP.fade(0.14 * ca));
    }
    g.icon(
        if a.chat_model_open { '\u{E721}' } else { '\u{E950}' },
        Rect::new(mdl_r.x + 5.0, mdl_r.y, 18.0, mdl_r.h),
        &f.icon14,
        C_UP.fade(ca),
    );
    let mdl_label = if a.chat_model_open {
        if a.chat_model_query.is_empty() {
            "Filter model\u{2026}\u{2502}".to_string()
        } else {
            format!("{}\u{2502}", a.chat_model_query)
        }
    } else {
        ellipsize(&a.chat_model, 17)
    };
    g.text(
        &mdl_label,
        Rect::new(mdl_r.x + 25.0, mdl_r.y, mdl_r.w - 44.0, mdl_r.h),
        &f.tiny,
        T1.fade(ca),
        Align::Left,
    );
    g.icon(
        '\u{E70D}',
        Rect::new(mdl_r.x + mdl_r.w - 20.0, mdl_r.y, 16.0, mdl_r.h),
        &f.icon14,
        (if a.chat_model_open { C_UP } else { T3 }).fade(ca),
    );

    // Copy last reply button
    let copy_r = Rect::new(EXP_W - 112.0, 2.0, 26.0, 22.0);
    hits.push(Hit {
        r: copy_r,
        act: Act::ChatCopyLast,
    });
    g.fill_rrect(copy_r, 11.0, (if hot(a, Act::ChatCopyLast) { HOVER } else { SURF }).fade(ca));
    g.icon(
        '\u{E8C8}',
        copy_r,
        &f.icon14,
        (if hot(a, Act::ChatCopyLast) { C_CPU } else { T2 }).fade(ca),
    );

    // Clear conversation button
    let clr_r = Rect::new(EXP_W - 80.0, 2.0, 26.0, 22.0);
    hits.push(Hit {
        r: clr_r,
        act: Act::ChatClear,
    });
    g.fill_rrect(clr_r, 11.0, (if hot(a, Act::ChatClear) { HOVER } else { SURF }).fade(ca));
    g.icon(
        '\u{E74D}',
        clr_r,
        &f.icon14,
        (if hot(a, Act::ChatClear) { T1 } else { T2 }).fade(ca),
    );

    // Sign out button
    let out_r = Rect::new(EXP_W - 48.0, 2.0, 24.0, 22.0);
    hits.push(Hit {
        r: out_r,
        act: Act::ChatSignOut,
    });
    g.fill_rrect(out_r, 11.0, (if hot(a, Act::ChatSignOut) { HOVER } else { SURF }).fade(ca));
    g.icon(
        '\u{E8F8}',
        out_r,
        &f.icon14,
        (if hot(a, Act::ChatSignOut) {
            Color(0.98, 0.45, 0.55, 1.0)
        } else {
            T2
        })
        .fade(ca),
    );

    // ---- Conversation Area (y = 28.0..144.0)
    let conv_r = Rect::new(24.0, 28.0, 552.0, 116.0);
    g.fill_rrect(conv_r, 18.0, SURF.fade(ca));

    if a.chat_msgs.is_empty() {
        let ic = Rect::new(conv_r.x + 20.0, conv_r.cy() - 17.0, 34.0, 34.0);
        g.fill_circle(ic.cx(), ic.cy(), 17.0, C_BAT.fade(0.18 * ca));
        g.icon('\u{E950}', ic, &f.icon22, C_BAT.fade(ca));

        g.text(
            "ChatGPT",
            Rect::new(conv_r.x + 66.0, conv_r.cy() - 18.0, conv_r.w - 82.0, 20.0),
            &f.body,
            T1.fade(ca),
            Align::Left,
        );
        let sub = if let Some(err) = &a.chat_error {
            err.as_str()
        } else {
            "Type a message below to start chatting with ChatGPT"
        };
        g.text(
            sub,
            Rect::new(conv_r.x + 66.0, conv_r.cy() + 4.0, conv_r.w - 82.0, 18.0),
            &f.tiny,
            (if a.chat_error.is_some() {
                Color(0.98, 0.45, 0.55, 1.0)
            } else {
                T3
            })
            .fade(ca),
            Align::Left,
        );
    } else {
        let has_target_tile = a.chat_active_doc.is_some();
        let lines = a.chat_wrapped_lines();
        let vis_lines = if has_target_tile { 3 } else { CHAT_VIS_LINES };
        let start = a.chat_scroll.min(lines.len().saturating_sub(vis_lines));
        let row_h = 21.0;
        for row in 0..vis_lines {
            let Some(line) = lines.get(start + row) else { break };
            let ry = conv_r.y + 6.0 + row as f32 * row_h;
            let is_user = line.role == crate::codex::Role::User;
            let (tint, glyph) = if is_user {
                (C_CPU, '\u{E77B}')
            } else {
                (C_BAT, '\u{E950}')
            };
            if line.first_of_turn {
                let ic = Rect::new(conv_r.x + 12.0, ry + 1.5, 18.0, 18.0);
                g.fill_circle(ic.cx(), ic.cy(), 9.0, tint.fade(0.18 * ca));
                g.icon(glyph, ic, &f.icon14, tint.fade(ca));
            }
            let tc = if is_user { C_CPU } else { T1 };
            let is_last_row = start + row + 1 == lines.len();
            let is_streaming_turn = a.chat_streaming && !is_user && is_last_row;

            let (display_text, text_col) = if is_streaming_turn {
                let cursor = if (a.spin_t * 5.0).fract() < 0.5 { "\u{2502}" } else { " " };
                if line.text.trim().is_empty() {
                    (format!("Thinking\u{2026} {cursor}"), C_BAT)
                } else if line.text.contains("```") || line.text.contains("save:") {
                    (format!("Generating contents\u{2026} Using MCP tools {cursor}"), C_BAT)
                } else {
                    (format!("{}{cursor}", line.text), tc)
                }
            } else {
                (line.text.clone(), tc)
            };

            let matched_doc = if !is_user && !is_streaming_turn {
                let trimmed = line.text.trim();
                a.chat_docs.iter().enumerate().find(|(_, p)| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.eq_ignore_ascii_case(trimmed))
                        .unwrap_or(false)
                })
            } else {
                None
            };

            if let Some((doc_idx, _path)) = matched_doc {
                let open_r = Rect::new(conv_r.x + conv_r.w - 104.0, ry + 1.0, 56.0, 19.0);
                let rev_r = Rect::new(conv_r.x + conv_r.w - 44.0, ry + 1.0, 32.0, 19.0);

                hits.push(Hit { r: open_r, act: Act::ChatOpenDoc(doc_idx) });
                let open_hot = hot(a, Act::ChatOpenDoc(doc_idx));
                g.fill_rrect(open_r, 9.5, (if open_hot { HOVER } else { Color::white(0.08) }).fade(ca));
                g.text(
                    "\u{2197} Open",
                    Rect::new(open_r.x, open_r.y + 1.5, open_r.w, open_r.h),
                    &f.tiny,
                    (if open_hot { T1 } else { C_CPU }).fade(ca),
                    Align::Center,
                );

                hits.push(Hit { r: rev_r, act: Act::ChatRevealDoc(doc_idx) });
                let rev_hot = hot(a, Act::ChatRevealDoc(doc_idx));
                g.fill_rrect(rev_r, 9.5, (if rev_hot { HOVER } else { Color::white(0.08) }).fade(ca));
                g.icon('\u{ED25}', rev_r, &f.icon14, (if rev_hot { T1 } else { T2 }).fade(ca));

                g.text(
                    &display_text,
                    Rect::new(conv_r.x + 38.0, ry, conv_r.w - 150.0, row_h),
                    &f.small,
                    C_CPU.fade(ca),
                    Align::Left,
                );
            } else {
                g.text(
                    &display_text,
                    Rect::new(conv_r.x + 38.0, ry, conv_r.w - 52.0, row_h),
                    &f.small,
                    text_col.fade(ca),
                    Align::Left,
                );
            }
        }
        if lines.len() > vis_lines {
            let (top, h) = (conv_r.y + 8.0, if has_target_tile { conv_r.h - 52.0 } else { conv_r.h - 16.0 });
            let track = Rect::new(conv_r.x + conv_r.w - 7.0, top, 3.0, h);
            g.fill_rrect(track, 1.5, Color::white(0.08 * ca));
            let th = (h * vis_lines as f32 / lines.len() as f32).max(16.0);
            let frac = start as f32 / (lines.len() - vis_lines) as f32;
            g.fill_rrect(
                Rect::new(track.x, top + (h - th) * frac.clamp(0.0, 1.0), 3.0, th),
                1.5,
                C_BAT.fade(0.8 * ca),
            );
        }

        let target_doc_view = a.chat_active_doc.as_ref().map(|d| {
            (
                d.slug.as_str(),
                d.app_name.as_str(),
                d.file_name.as_str(),
                &d.file_path,
            )
        });

        if let Some((slug, app_name, file_name, file_path)) = target_doc_view {
            let tile_r = Rect::new(conv_r.x + 8.0, conv_r.y + conv_r.h - 44.0, conv_r.w - 16.0, 38.0);
            let is_edit = a.chat_doc_mode == crate::craft::ChatDocMode::EditExisting;
            let (tint, glyph) = match slug {
                "deckcraft" => (C_MEM, '\u{E8A5}'),
                "gridcraft" => (C_BAT, '\u{E8EC}'),
                "cadcraft" => (Color(0.98, 0.45, 0.55, 1.0), '\u{E821}'),
                _ => (C_CPU, '\u{E8A5}'),
            };

            g.fill_rrect(tile_r, 14.0, Color::white(0.10).fade(ca));

            // Circular badge
            let ic = Rect::new(tile_r.x + 8.5, tile_r.y + (tile_r.h - 25.0) / 2.0, 25.0, 25.0);
            g.fill_circle(ic.cx(), ic.cy(), 12.5, tint.fade(0.18 * ca));
            g.icon(glyph, ic, &f.icon14, tint.fade(ca));

            // Text on tile:
            let tx_w = tile_r.w - 42.0 - 188.0;
            g.text(
                &ellipsize(file_name, 32),
                Rect::new(tile_r.x + 42.0, tile_r.y + 4.0, tx_w, 16.0),
                &f.small,
                T1.fade(ca),
                Align::Left,
            );
            let status_text = if is_edit {
                format!("{app_name} \u{00B7} Target: Editing file in-place")
            } else {
                format!("{app_name} \u{00B7} Target: Creating new document")
            };
            g.text(
                &status_text,
                Rect::new(tile_r.x + 42.0, tile_r.y + 20.0, tx_w, 14.0),
                &f.tiny,
                (if is_edit { C_CPU } else { C_BAT }).fade(ca),
                Align::Left,
            );

            // Right interactive pills:
            // 1. "Edit" pill
            let edit_r = Rect::new(tile_r.x + tile_r.w - 192.0, tile_r.y + 7.0, 50.0, 24.0);
            hits.push(Hit {
                r: edit_r,
                act: Act::ChatSetDocMode(crate::craft::ChatDocMode::EditExisting),
            });
            let edit_hot = hot(a, Act::ChatSetDocMode(crate::craft::ChatDocMode::EditExisting));
            g.fill_rrect(
                edit_r,
                12.0,
                (if is_edit {
                    C_CPU.fade(0.28)
                } else if edit_hot {
                    HOVER
                } else {
                    Color::white(0.06)
                })
                .fade(ca),
            );
            g.text(
                if is_edit { "Edit \u{2713}" } else { "Edit" },
                edit_r,
                &f.tiny,
                (if is_edit { C_CPU } else if edit_hot { T1 } else { T2 }).fade(ca),
                Align::Center,
            );

            // 2. "+ New" pill
            let new_r = Rect::new(tile_r.x + tile_r.w - 138.0, tile_r.y + 7.0, 54.0, 24.0);
            hits.push(Hit {
                r: new_r,
                act: Act::ChatSetDocMode(crate::craft::ChatDocMode::CreateNew),
            });
            let new_hot = hot(a, Act::ChatSetDocMode(crate::craft::ChatDocMode::CreateNew));
            g.fill_rrect(
                new_r,
                12.0,
                (if !is_edit {
                    C_BAT.fade(0.28)
                } else if new_hot {
                    HOVER
                } else {
                    Color::white(0.06)
                })
                .fade(ca),
            );
            g.text(
                if !is_edit { "+ New \u{2713}" } else { "+ New" },
                new_r,
                &f.tiny,
                (if !is_edit { C_BAT } else if new_hot { T1 } else { T2 }).fade(ca),
                Align::Center,
            );

            // 3. Open & Reveal buttons
            if let Some(doc_idx) = a.chat_docs.iter().position(|p| p == file_path) {
                // Open directly in app
                let open_r = Rect::new(tile_r.x + tile_r.w - 80.0, tile_r.y + 7.0, 24.0, 24.0);
                hits.push(Hit {
                    r: open_r,
                    act: Act::ChatOpenDoc(doc_idx),
                });
                let open_hot = hot(a, Act::ChatOpenDoc(doc_idx));
                g.fill_rrect(open_r, 12.0, (if open_hot { HOVER } else { Color::white(0.06) }).fade(ca));
                g.icon('\u{E8A7}', open_r, &f.icon14, (if open_hot { T1 } else { T2 }).fade(ca));

                // Reveal in File Explorer
                let rev_r = Rect::new(tile_r.x + tile_r.w - 52.0, tile_r.y + 7.0, 24.0, 24.0);
                hits.push(Hit {
                    r: rev_r,
                    act: Act::ChatRevealDoc(doc_idx),
                });
                let rev_hot = hot(a, Act::ChatRevealDoc(doc_idx));
                g.fill_rrect(rev_r, 12.0, (if rev_hot { HOVER } else { Color::white(0.06) }).fade(ca));
                g.icon('\u{ED25}', rev_r, &f.icon14, (if rev_hot { T1 } else { T2 }).fade(ca));
            }

            // 4. Detach / Clear target
            let clear_r = Rect::new(tile_r.x + tile_r.w - 24.0, tile_r.y + 7.0, 20.0, 24.0);
            hits.push(Hit {
                r: clear_r,
                act: Act::ChatClearTargetDoc,
            });
            let clear_hot = hot(a, Act::ChatClearTargetDoc);
            g.icon('\u{E711}', clear_r, &f.icon14, (if clear_hot { T1 } else { T3 }).fade(ca));
        }
    }

    // ---- Bottom Prompt Composer Card (x = 24..576, y = 150.0, h = 50.0)
    let comp_r = Rect::new(24.0, 150.0, 552.0, 50.0);
    let typing = a.entry_chat && a.entering.is_some();

    let editor_lines: Vec<String> = if typing {
        let raw: Vec<char> = a.chat_draft.chars().collect();
        let cur = a.chat_cursor.min(raw.len());
        let mut text_with_caret = String::with_capacity(raw.len() + 2);
        text_with_caret.extend(&raw[..cur]);
        let blink = (a.spin_t * 5.0).fract() < 0.5;
        text_with_caret.push(if blink { '\u{2502}' } else { ' ' });
        text_with_caret.extend(&raw[cur..]);

        let mut lines = Vec::new();
        for l in text_with_caret.lines() {
            lines.push(l.to_string());
        }
        if lines.is_empty() {
            lines.push(format!("{}", if blink { "\u{2502}" } else { " " }));
        }
        lines
    } else if !a.chat_draft.is_empty() {
        a.chat_draft.lines().map(|s| s.to_string()).collect()
    } else {
        vec!["Message ChatGPT\u{2026}".to_string()]
    };

    let prompt_col = if typing && a.chat_draft.is_empty() {
        T2
    } else if !a.chat_draft.is_empty() || typing {
        T1
    } else {
        T2
    };

    let (sub_str, sub_col) = if let Some(err) = &a.chat_error {
        (ellipsize(err, 60), Color(0.98, 0.45, 0.55, 1.0))
    } else if let Some(att) = &a.chat_attachment {
        (
            format!("Attached: {} ({}) \u{00B7} Enter to send", ellipsize(&att.name, 22), att.size_label()),
            C_MEM,
        )
    } else if a.chat_streaming {
        (
            "Streaming realtime response\u{2026} \u{00B7} Click \u{25A0} to stop".to_string(),
            C_BAT,
        )
    } else if typing {
        if let Some(doc) = &a.chat_active_doc {
            if a.chat_doc_mode == crate::craft::ChatDocMode::EditExisting {
                (
                    format!("Editing {} \u{00B7} Shift+Enter \u{21B5} Line \u{00B7} Enter \u{21B5} Send", ellipsize(&doc.file_name, 18)),
                    C_CPU,
                )
            } else {
                (
                    format!("Creating new document \u{00B7} Shift+Enter \u{21B5} Line \u{00B7} Enter \u{21B5} Send"),
                    C_BAT,
                )
            }
        } else {
            (
                "Shift+Enter \u{21B5} Line \u{00B7} Enter \u{21B5} Send \u{00B7} Ctrl+V Paste".to_string(),
                C_CPU,
            )
        }
    } else {
        if let Some(doc) = &a.chat_active_doc {
            if a.chat_doc_mode == crate::craft::ChatDocMode::EditExisting {
                (
                    format!("Target: {} (Edit mode) \u{00B7} Click to type", ellipsize(&doc.file_name, 22)),
                    C_CPU,
                )
            } else {
                (
                    format!("Target: {} (+New mode) \u{00B7} Click to type", ellipsize(&doc.file_name, 22)),
                    C_BAT,
                )
            }
        } else {
            (
                format!("Model: {} \u{00B7} Click to type a prompt", a.chat_model),
                T3,
            )
        }
    };

    let k = hot_k(a, Act::ChatPrompt);
    if typing {
        g.fill_rrect(comp_r, 18.0, HOVER.fade(ca));
        g.fill_rrect(comp_r, 18.0, C_CPU.fade(0.14 * ca));
    } else {
        g.fill_rrect(comp_r, 18.0, Color::white((0.09 + 0.08 * k) * ca));
    }
    hits.push(Hit {
        r: Rect::new(comp_r.x, comp_r.y, comp_r.w - 114.0, comp_r.h),
        act: Act::ChatPrompt,
    });

    let ic = Rect::new(comp_r.x + 12.0, comp_r.y + (comp_r.h - 34.0) / 2.0, 34.0, 34.0);
    let prompt_tint = if a.chat_streaming {
        C_BAT
    } else if typing {
        C_CPU
    } else if a.chat_active_doc.is_some() {
        C_MEM
    } else {
        C_CPU
    };
    g.fill_circle(ic.cx(), ic.cy(), 17.0, prompt_tint.fade(0.16 * ca));
    let editor_glyph = if typing { '\u{E70F}' } else { '\u{E8BD}' };
    g.icon(editor_glyph, ic, &f.icon14, prompt_tint.fade(ca));

    let tw = comp_r.w - 56.0 - 116.0;
    if editor_lines.len() >= 2 {
        let line_count = editor_lines.len();
        let line_a = &editor_lines[line_count - 2];
        let line_b = &editor_lines[line_count - 1];
        g.text(line_a, Rect::new(comp_r.x + 56.0, comp_r.y + 6.0, tw, 18.0), &f.body, prompt_col.fade(ca), Align::Left);
        g.text(line_b, Rect::new(comp_r.x + 56.0, comp_r.y + 24.0, tw, 18.0), &f.body, prompt_col.fade(ca), Align::Left);
    } else {
        let ty = comp_r.y + (comp_r.h - 43.0) / 2.0;
        let line_0 = editor_lines.first().map(|s| s.as_str()).unwrap_or("");
        g.text(line_0, Rect::new(comp_r.x + 56.0, ty, tw, 22.0), &f.body, prompt_col.fade(ca), Align::Left);
        g.text(&sub_str, Rect::new(comp_r.x + 56.0, ty + 23.0, tw, 18.0), &f.tiny, sub_col.fade(ca), Align::Left);
    }

    // Button 1: Attach File
    let att_r = Rect::new(comp_r.x + comp_r.w - 108.0, comp_r.y + 11.0, 30.0, 30.0);
    let att_act = if a.chat_attachment.is_some() {
        Act::ChatRemoveAttachment
    } else {
        Act::ChatAttachFile
    };
    hits.push(Hit { r: att_r, act: att_act });
    let att_hot = hot(a, att_act);
    let att_has = a.chat_attachment.is_some();
    g.fill_circle(
        att_r.cx(),
        att_r.cy(),
        15.0,
        (if att_has {
            C_MEM.fade(0.25)
        } else if att_hot {
            HOVER
        } else {
            Color::white(0.06)
        })
        .fade(ca),
    );
    g.icon(
        if att_has { '\u{E711}' } else { '\u{E723}' },
        att_r,
        &f.icon14,
        (if att_has { C_MEM } else if att_hot { T1 } else { T2 }).fade(ca),
    );

    // Button 2: Speech to Text (Microphone)
    let mic_r = Rect::new(comp_r.x + comp_r.w - 72.0, comp_r.y + 11.0, 30.0, 30.0);
    hits.push(Hit {
        r: mic_r,
        act: Act::ChatSpeechToText,
    });
    let mic_hot = hot(a, Act::ChatSpeechToText);
    g.fill_circle(
        mic_r.cx(),
        mic_r.cy(),
        15.0,
        (if mic_hot { HOVER } else { Color::white(0.06) }).fade(ca),
    );
    g.icon(
        '\u{E720}',
        mic_r,
        &f.icon14,
        (if mic_hot { C_CPU } else { T2 }).fade(ca),
    );

    // Button 3: Send or Stop
    let send_r = Rect::new(comp_r.x + comp_r.w - 36.0, comp_r.y + 11.0, 30.0, 30.0);
    if a.chat_streaming {
        hits.push(Hit { r: send_r, act: Act::ChatStop });
        let pulse = 0.5 + 0.5 * (a.spin_t * 6.0).sin();
        let sc = Color(0.98, 0.45, 0.55, 1.0);
        g.fill_circle(send_r.cx(), send_r.cy(), 15.0, sc.fade((0.18 + 0.18 * pulse) * ca));
        g.icon('\u{E71A}', send_r, &f.icon14, sc.fade(ca));
        spinner(g, comp_r.x + 29.0, comp_r.cy(), 16.5, a.spin_t, C_BAT, ca);
    } else {
        let has_input = !a.chat_draft.trim().is_empty() || a.chat_attachment.is_some();
        if has_input {
            hits.push(Hit { r: send_r, act: Act::ChatSend });
            let send_hot = hot(a, Act::ChatSend);
            g.fill_circle(
                send_r.cx(),
                send_r.cy(),
                15.0,
                C_CPU.fade((if send_hot { 0.32 } else { 0.18 }) * ca),
            );
            g.icon('\u{E724}', send_r, &f.icon14, C_CPU.fade(ca));
        } else {
            g.fill_circle(send_r.cx(), send_r.cy(), 15.0, Color::white(0.05 * ca));
            g.icon('\u{E724}', send_r, &f.icon14, T3.fade(ca));
        }
    }

    // ---- Searchable Model Dropdown Overlay (drawn last so it floats cleanly above conversation)
    if a.chat_model_open {
        hits.push(Hit {
            r: Rect::new(0.0, 0.0, EXP_W, EXP_H - CONTENT_Y),
            act: Act::ChatCloseDrop,
        });
        hits.push(Hit {
            r: mdl_r,
            act: Act::ChatModelDrop,
        });

        let opts = a.chat_model_options();
        let row_h = 22.0;
        let vis_rows = opts.len().clamp(1, CHAT_MODEL_DROP_ROWS);
        let drop_h = 6.0 + vis_rows as f32 * row_h;
        let drop_r = Rect::new(306.0, 27.0, 270.0, drop_h);
        g.fill_rrect(drop_r, 16.0, BLACK.fade(ca));
        g.fill_rrect(drop_r, 16.0, Color::white(0.12 * ca));

        if opts.is_empty() {
            g.text("No matching models", drop_r, &f.small, T3.fade(ca), Align::Center);
        } else {
            for row in 0..CHAT_MODEL_DROP_ROWS {
                let idx = a.chat_model_scroll + row;
                let Some(m) = opts.get(idx) else { break };
                let rr = Rect::new(
                    drop_r.x + 4.0,
                    drop_r.y + 3.0 + row as f32 * row_h,
                    drop_r.w - 12.0,
                    row_h - 1.0,
                );
                let oact = Act::ChatPickModel(idx);
                hits.push(Hit { r: rr, act: oact });
                let is_sel = idx == a.chat_model_sel;
                let is_hot = hot(a, oact);
                if is_hot || is_sel {
                    g.fill_rrect(rr, 9.0, Color::white((if is_hot { 0.16 } else { 0.10 }) * ca));
                }
                let active = a.chat_model.eq_ignore_ascii_case(&m.slug);
                g.fill_circle(rr.x + 11.0, rr.cy(), 4.0, (if active { C_UP } else { T3 }).fade(ca));
                g.text(
                    &m.slug,
                    Rect::new(rr.x + 22.0, rr.y, rr.w - 46.0, rr.h),
                    &f.small,
                    T1.fade(ca),
                    Align::Left,
                );
                if active {
                    g.icon(
                        '\u{E73E}',
                        Rect::new(rr.x + rr.w - 22.0, rr.y, 18.0, rr.h),
                        &f.icon14,
                        C_UP.fade(ca),
                    );
                }
            }
            if opts.len() > CHAT_MODEL_DROP_ROWS {
                let (top, h) = (drop_r.y + 6.0, drop_r.h - 12.0);
                let track = Rect::new(drop_r.x + drop_r.w - 6.0, top, 3.0, h);
                g.fill_rrect(track, 1.5, Color::white(0.08 * ca));
                let th = (h * CHAT_MODEL_DROP_ROWS as f32 / opts.len() as f32).max(14.0);
                let frac = a.chat_model_scroll as f32 / (opts.len() - CHAT_MODEL_DROP_ROWS) as f32;
                g.fill_rrect(
                    Rect::new(track.x, top + (h - th) * frac.clamp(0.0, 1.0), 3.0, th),
                    1.5,
                    C_UP.fade(0.8 * ca),
                );
            }
        }
    }
}

fn draw_calendar(a: &App, ca: f32) {
    let g = &a.gfx;
    let f = &g.fonts;
    let (year, month, day, wday, ..) = sys::local_time();
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let x0 = 432.0;
    let cw = 20.0;
    g.text(
        &format!("{} {}", MONTHS[(month as usize - 1).min(11)], year),
        Rect::new(x0 - 4.0, 12.0, cw * 7.0 + 8.0, 22.0),
        &f.small,
        C_UP.fade(ca),
        Align::Center,
    );
    for (i, d) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
        g.text(
            d,
            Rect::new(x0 + i as f32 * cw, 36.0, cw, 18.0),
            &f.tiny,
            C_UP.fade(0.55 * ca),
            Align::Center,
        );
    }
    let first = first_weekday(day, wday);
    let dim = days_in_month(year, month);
    for d in 1..=dim {
        let slot = first + d - 1;
        let (c, r) = (slot % 7, slot / 7);
        let cell = Rect::new(x0 + c as f32 * cw, 56.0 + r as f32 * 20.0, cw, 20.0);
        if d == day {
            g.fill_circle(cell.cx(), cell.cy(), 9.5, C_UP.fade(ca));
            g.text(
                &d.to_string(),
                cell,
                &f.tiny,
                Color(0.0, 0.0, 0.0, ca),
                Align::Center,
            );
        } else {
            let col = if c == 0 || c == 6 { T3 } else { T2 };
            g.text(&d.to_string(), cell, &f.tiny, col.fade(ca), Align::Center);
        }
    }
}

pub(crate) use crate::cal::{days_in_month, first_of_month, first_weekday};

// --------------------------------------------------------------- helpers ----

fn slider(
    a: &App,
    r: Rect,
    frac: f32,
    act: Act,
    fill: Color,
    ca: f32,
    hits: &mut Vec<Hit>,
    enabled: bool,
) {
    let g = &a.gfx;
    let track = Rect::new(r.x, r.cy() - 3.0, r.w, 6.0);
    g.fill_rrect(track, 3.0, Color::white(0.16 * ca));
    let fw = (r.w * frac.clamp(0.0, 1.0)).max(0.0);
    // progress shows even when the app does not allow seeking; only the knob and clicks need `enabled`
    if fw > 0.5 {
        g.fill_rrect(Rect::new(r.x, track.y, fw, 6.0), 3.0, fill.fade(ca));
    }
    if enabled {
        hits.push(Hit { r, act });
        if hot(a, act) || a.seek_frac.is_some() && act == Act::Seek {
            g.fill_circle(r.x + fw, r.cy(), 7.0, Color::white(ca));
        }
    }
}

pub use crate::cal::fmt_time;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_fraction_is_clamped() {
        assert_eq!(focus_fraction(1500, 1_500_000), 0.0);
        assert!((focus_fraction(1500, 750_000) - 0.5).abs() < 1e-6);
        assert_eq!(focus_fraction(1500, 0), 1.0);
        assert_eq!(focus_fraction(1500, -5), 1.0);
        assert_eq!(focus_fraction(0, 0), 1.0);
    }

    #[test]
    fn previews_collapse_whitespace_and_truncate() {
        assert_eq!(preview("a \n\t b   c"), "a b c");
        assert_eq!(preview(&"x".repeat(500)).chars().count(), 160);
    }

    #[test]
    fn eleven_tabs_fit_between_pin_and_collapse() {
        assert_eq!(TAB_LABELS.len(), PAGES.len());
        let rects = tab_rects();
        assert_eq!(rects.len(), 11);
        assert!(rects[0].x >= 42.0, "first tab overlaps pin: {}", rects[0].x);
        let last = rects.last().unwrap();
        assert!(last.x + last.w <= EXP_W - 42.0, "last tab overlaps collapse: {}", last.x + last.w);
        for pair in rects.windows(2) {
            assert!(pair[1].x >= pair[0].x + pair[0].w + 1.5);
        }
    }
}
