//! Icons drawn from one set of shapes in three looks: Line, Duotone and Solid.
//!
//! Each icon is a list of parts on a 24-unit grid, written as SVG path data
//! (plus `O cx cy r` for a circle). Every part has a role that the look turns
//! into pixels:
//!
//! | role        | Line    | Duotone              | Solid             |
//! |-------------|---------|----------------------|-------------------|
//! | stroke      | line    | line                 | line              |
//! | body        | outline | outline + light tint | filled            |
//! | fill        | filled  | filled               | filled            |
//! | detail      | line    | line                 | cut out (a hole)  |
//! | detail fill | filled  | filled               | cut out           |
//!
//! An icon becomes a white coverage mask at the exact pixel size it is shown
//! at, cached as a texture and tinted when drawn, so every look works in any
//! color and stays sharp at any display scale.
use super::style::IconLook;
use ab_glyph_rasterizer::{Rasterizer, point};
use eframe::egui::{self, Color32, Pos2, Rect, vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glyph {
    Pencil,
    Pen,
    Brush,
    Airbrush,
    Wet,
    Blend,
    Pixel,
    Eraser,
    Fill,
    Gradient,
    Picker,
    Select,
    Lasso,
    LassoErase,
    Move,
    Text,
    Pointer,
    Layers,
    Undo,
    Redo,
    Eye,
    EyeOff,
    Lock,
    Plus,
    Trash,
    Duplicate,
    MergeDown,
    Folder,
    NewFolder,
    Mask,
    Clip,
    AlphaLock,
    Effects,
    Adjustment,
    More,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    ZoomIn,
    ZoomOut,
    RotateLeft,
    RotateRight,
    Flip,
    Fit,
    Symmetry,
    Global,
    Guide,
    Play,
    Onion,
    Search,
    Grid,
    Pressure,
    Snap,
    Menu,
    Sliders,
}

#[cfg(test)]
impl Glyph {
    pub const ALL: [Glyph; 55] = [
        Glyph::Pencil,
        Glyph::Pen,
        Glyph::Brush,
        Glyph::Airbrush,
        Glyph::Wet,
        Glyph::Blend,
        Glyph::Pixel,
        Glyph::Eraser,
        Glyph::Fill,
        Glyph::Gradient,
        Glyph::Picker,
        Glyph::Select,
        Glyph::Lasso,
        Glyph::LassoErase,
        Glyph::Move,
        Glyph::Text,
        Glyph::Pointer,
        Glyph::Layers,
        Glyph::Undo,
        Glyph::Redo,
        Glyph::Eye,
        Glyph::EyeOff,
        Glyph::Lock,
        Glyph::Plus,
        Glyph::Trash,
        Glyph::Duplicate,
        Glyph::MergeDown,
        Glyph::Folder,
        Glyph::NewFolder,
        Glyph::Mask,
        Glyph::Clip,
        Glyph::AlphaLock,
        Glyph::Effects,
        Glyph::Adjustment,
        Glyph::More,
        Glyph::ChevronRight,
        Glyph::ChevronDown,
        Glyph::ChevronUp,
        Glyph::ZoomIn,
        Glyph::ZoomOut,
        Glyph::RotateLeft,
        Glyph::RotateRight,
        Glyph::Flip,
        Glyph::Fit,
        Glyph::Symmetry,
        Glyph::Global,
        Glyph::Guide,
        Glyph::Play,
        Glyph::Onion,
        Glyph::Search,
        Glyph::Grid,
        Glyph::Pressure,
        Glyph::Snap,
        Glyph::Menu,
        Glyph::Sliders,
    ];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Stroke,
    Body,
    Fill,
    Detail,
    DetailFill,
}

/// Which looks draw a part.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Only {
    All,
    /// Line and Duotone.
    Outline,
    Solid,
}

#[derive(Clone, Copy, Debug)]
struct Part {
    role: Role,
    path: &'static str,
    only: Only,
    dash: Option<[f32; 2]>,
    opacity: f32,
}

const fn part(role: Role, path: &'static str) -> Part {
    Part {
        role,
        path,
        only: Only::All,
        dash: None,
        opacity: 1.0,
    }
}
const fn s(path: &'static str) -> Part {
    part(Role::Stroke, path)
}
const fn b(path: &'static str) -> Part {
    part(Role::Body, path)
}
const fn f(path: &'static str) -> Part {
    part(Role::Fill, path)
}
const fn k(path: &'static str) -> Part {
    part(Role::Detail, path)
}
const fn kf(path: &'static str) -> Part {
    part(Role::DetailFill, path)
}
impl Part {
    const fn dash(mut self, on: f32, off: f32) -> Self {
        self.dash = Some([on, off]);
        self
    }
    const fn only(mut self, only: Only) -> Self {
        self.only = only;
        self
    }
    const fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
}

const LOOP: &str =
    "M12 4C17.4 4 21 6.6 21 9.8 21 13 17.4 15.4 12.4 15.4 7 15.4 3 13 3 9.8 3 6.6 6.6 4 12 4Z";
const TAIL: &str = "M7.6 14.6C6.6 16.4 7.6 18 9.6 18 11.2 18 11.6 19.6 10.6 21";
const EYE: &str = "M2.5 12C5 7 8.5 5 12 5S19 7 21.5 12C19 17 15.5 19 12 19S5 17 2.5 12Z";
const FOLDER: &str = "M3 6H9.5L11.5 8H21V19H3Z";
const CHECKER: [Part; 2] = [s("M4 4H20V20H4Z"), f("M4 4H12V12H4ZM12 12H20V20H12Z")];

fn parts(glyph: Glyph) -> &'static [Part] {
    match glyph {
        Glyph::Pencil => {
            const {
                &[
                    b("M15.5 4.5 19.5 8.5 9 19 5 15Z"),
                    b("M5 15 9 19 3.5 20.5Z"),
                    kf("M3.5 20.5 4.2 17.9 6.1 19.8Z"),
                    k("M13.5 6.5 17.5 10.5"),
                ]
            }
        }
        Glyph::Pen => {
            const {
                &[
                    b("M4 20 7.5 11 13 5.5Q15 3.5 17.5 6L18 6.5Q20 8.5 18 10.5L13 16.5Z"),
                    k("M4 20 10.4 13.6"),
                    kf("O12.6 11.4 1.3"),
                ]
            }
        }
        Glyph::Brush => {
            const {
                &[
                    s("M21 3 15.2 8.8"),
                    b("M13.2 7.8 16.2 10.8 14.2 12.8 11.2 9.8Z"),
                    b("M11.2 9.8 14.2 12.8C13 17.6 8.6 20.6 3 21 3.4 15.4 6.4 11 11.2 9.8Z"),
                ]
            }
        }
        Glyph::Airbrush => {
            const {
                &[
                    b("M19.6 2.8 21.2 4.4 12.2 13.4 10.6 11.8Z"),
                    b("O15.4 5.2 2.1"),
                    s("M16.8 8.6 18.9 10.7"),
                    f("M10.6 11.8 12.2 13.4 9.6 14.4Z"),
                    f("O7 16.4 1.1O4.6 17.4 0.9O6.4 19.8 0.9O3.4 20.6 0.75O8.8 19.2 0.7"),
                ]
            }
        }
        Glyph::Wet => {
            const {
                &[
                    b("M12 3C15.6 7.6 18 10.8 18 14.2A6 6 0 0 1 6 14.2C6 10.8 8.4 7.6 12 3Z"),
                    k("M9.2 14.6A3 3 0 0 0 11.6 17.4"),
                ]
            }
        }
        Glyph::Blend => {
            const {
                &[
                    b(
                        "M3 17C6 12 9 12 12 14C15 16 18 13 21 7.5 20.4 14 16.5 19.2 12 18.4 9 17.8 6.6 16.6 3 17Z",
                    ),
                    s("M5.5 8C8 6.4 10.6 6.6 12.6 8"),
                ]
            }
        }
        Glyph::Pixel => const { &[f("M4 16h4v4H4zM8 12h4v4H8zM12 8h4v4h-4zM16 4h4v4h-4z")] },
        Glyph::Eraser => {
            const {
                &[
                    b("M4 14 12 6 18 12 10 20Z"),
                    k("M7.2 10.8 13.2 16.8"),
                    s("M14 20.5H20.5"),
                ]
            }
        }
        Glyph::Fill => {
            const {
                &[
                    b("M5 11 11 5 18 12 12 18Z"),
                    k("M6.5 12.5H17.5"),
                    f(
                        "M19.6 14C20.9 15.8 21.6 16.9 21.6 17.8A2 2 0 0 1 17.6 17.8C17.6 16.9 18.3 15.8 19.6 14Z",
                    ),
                ]
            }
        }
        Glyph::Gradient => const { &[s("M3.5 5.5H20.5V18.5H3.5Z"), f("M5.5 16.5H18.5V7.5Z")] },
        Glyph::Picker => {
            const {
                &[
                    b("O17.5 6.5 3"),
                    s("M12.6 7.6 16.4 11.4"),
                    b("M15.3 10.1 6.5 18.9 5.1 17.5 13.9 8.7Z"),
                    f("O4.4 19.6 1.4"),
                ]
            }
        }
        Glyph::Select => const { &[s("M4 5H20V19H4Z").dash(2.6, 2.4)] },
        Glyph::Lasso => const { &[b(LOOP), s(TAIL)] },
        Glyph::LassoErase => const { &[s(LOOP), s(TAIL), s("M8.5 9.8H15.5")] },
        Glyph::Move => {
            const {
                &[
                    s("M12 3V21M3 12H21"),
                    s(
                        "M9.5 5.5 12 3 14.5 5.5M9.5 18.5 12 21 14.5 18.5M5.5 9.5 3 12 5.5 14.5M18.5 9.5 21 12 18.5 14.5",
                    ),
                ]
            }
        }
        Glyph::Text => {
            const {
                &[
                s("M5 7.5V5H19V7.5M12 5V19M9 19H15").only(Only::Outline),
                b("M4.5 4.5H19.5V8.2H17.7L17.2 6.8H13.7V17.9L15.5 18.3V19.5H8.5V18.3L10.3 17.9V6.8H6.8L6.3 8.2H4.5Z")
                    .only(Only::Solid),
            ]
            }
        }
        Glyph::Pointer => const { &[b("M6 3.5V18.5L10 14.8 12.8 20.6 15.4 19.4 12.6 13.6H18Z")] },
        Glyph::Layers => {
            const {
                &[
                    b("M12 4 21 8.5 12 13 3 8.5Z"),
                    s("M3 12.5 12 17 21 12.5"),
                    s("M3 16.5 12 21 21 16.5"),
                ]
            }
        }
        Glyph::Undo => {
            const {
                &[
                    s("M8.5 5.5 4.5 9.5 8.5 13.5"),
                    s("M4.5 9.5H14A5 5 0 0 1 14 19.5H9"),
                ]
            }
        }
        Glyph::Redo => {
            const {
                &[
                    s("M15.5 5.5 19.5 9.5 15.5 13.5"),
                    s("M19.5 9.5H10A5 5 0 0 0 10 19.5H15"),
                ]
            }
        }
        Glyph::Eye => const { &[b(EYE), kf("O12 12 3")] },
        Glyph::EyeOff => const { &[b(EYE), k("M4 20 20 4")] },
        Glyph::Lock => {
            const {
                &[
                    s("M8 11V8A4 4 0 0 1 16 8V11"),
                    b("M5.5 11H18.5V20H5.5Z"),
                    k("M12 14.5V16.5"),
                ]
            }
        }
        Glyph::Plus => const { &[s("M12 5V19M5 12H19")] },
        Glyph::Trash => {
            const {
                &[
                    s("M4 6.5H20M9.5 6.5V4H14.5V6.5"),
                    b("M6 6.5H18L17 20H7Z"),
                    k("M10 10V16.5M14 10V16.5"),
                ]
            }
        }
        Glyph::Duplicate => const { &[b("M4 8H15V20H4Z"), s("M8 8V4H20V16H15")] },
        Glyph::MergeDown => {
            const {
                &[
                    s("M12 3V14"),
                    s("M8 10.5 12 14.5 16 10.5"),
                    b("M4 17.5H20V21H4Z"),
                ]
            }
        }
        Glyph::Folder => const { &[b(FOLDER)] },
        Glyph::NewFolder => const { &[b(FOLDER), k("M12 10.5V16.5M9 13.5H15")] },
        Glyph::Mask => const { &[b("M4 5H20V19H4Z"), kf("O12 12 4")] },
        Glyph::Clip => const { &[s("M7 3V15H19"), s("M15.5 11.5 19 15 15.5 18.5")] },
        Glyph::AlphaLock => &CHECKER,
        Glyph::Effects => {
            const {
                &[
                    b(
                        "M11 4C11.8 9 13 11.2 18 12.5 13 13.8 11.8 16 11 21 10.2 16 9 13.8 4 12.5 9 11.2 10.2 9 11 4Z",
                    ),
                    b(
                        "M18.5 2.5C18.8 4 19.2 4.4 20.5 4.7 19.2 5 18.8 5.4 18.5 6.9 18.2 5.4 17.8 5 16.5 4.7 17.8 4.4 18.2 4 18.5 2.5Z",
                    ),
                ]
            }
        }
        Glyph::Adjustment => const { &[s("O12 12 8.5"), f("M12 3.5A8.5 8.5 0 0 0 12 20.5Z")] },
        Glyph::More => const { &[f("O5.5 12 1.6O12 12 1.6O18.5 12 1.6")] },
        Glyph::ChevronRight => const { &[s("M9 6 15 12 9 18")] },
        Glyph::ChevronDown => const { &[s("M6 9 12 15 18 9")] },
        Glyph::ChevronUp => const { &[s("M6 15 12 9 18 15")] },
        Glyph::ZoomIn => {
            const {
                &[
                    b("O10.5 10.5 6.5"),
                    s("M15.5 15.5 20.5 20.5"),
                    k("M7.5 10.5H13.5M10.5 7.5V13.5"),
                ]
            }
        }
        Glyph::ZoomOut => {
            const {
                &[
                    b("O10.5 10.5 6.5"),
                    s("M15.5 15.5 20.5 20.5"),
                    k("M7.5 10.5H13.5"),
                ]
            }
        }
        Glyph::RotateLeft => {
            const {
                &[
                    s("M4.5 12A7.5 7.5 0 1 0 12 4.5H8"),
                    s("M10.5 2 8 4.5 10.5 7"),
                ]
            }
        }
        Glyph::RotateRight => {
            const {
                &[
                    s("M19.5 12A7.5 7.5 0 1 1 12 4.5H16"),
                    s("M13.5 2 16 4.5 13.5 7"),
                ]
            }
        }
        Glyph::Flip => {
            const {
                &[
                    s("M12 3V21").dash(2.0, 2.0),
                    s("M9.5 6V18H3.5Z"),
                    f("M14.5 6V18H20.5Z"),
                ]
            }
        }
        Glyph::Fit => const { &[s("M4 9V4H9M15 4H20V9M20 15V20H15M9 20H4V15")] },
        Glyph::Symmetry => {
            const {
                &[
                    s("M12 3V21").dash(2.0, 2.0),
                    b("O6.6 12 3.3"),
                    b("O17.4 12 3.3"),
                ]
            }
        }
        Glyph::Global => {
            const {
                &[
                    b("O12 12 8.5"),
                    k("M3.5 12H20.5"),
                    k(
                        "M12 3.5C9.5 6 8.5 9 8.5 12S9.5 18 12 20.5C14.5 18 15.5 15 15.5 12S14.5 6 12 3.5Z",
                    ),
                ]
            }
        }
        Glyph::Guide => {
            const {
                &[
                    b("M3 16.5 16.5 3 21 7.5 7.5 21Z"),
                    k("M7 12.5 9 14.5M9.8 9.7 11.8 11.7M12.6 6.9 14.6 8.9"),
                ]
            }
        }
        Glyph::Play => const { &[b("M8 5 19 12 8 19Z")] },
        Glyph::Onion => {
            const {
                &[
                    s("M7.6 6.6A6 6 0 0 0 7.6 17.4").opacity(0.6),
                    s("M4.2 8A6 6 0 0 0 4.2 16").opacity(0.35),
                    b("O14 12 6"),
                ]
            }
        }
        Glyph::Search => const { &[s("O10.5 10.5 6"), s("M15 15 20 20")] },
        Glyph::Grid => const { &[b("M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z")] },
        Glyph::Pressure => const { &[s("M4 4V20H20").opacity(0.45), s("M4 20C11 20 12 6 20 4")] },
        Glyph::Snap => {
            const {
                &[
                    s("M5 4V11A7 7 0 0 0 19 11V4"),
                    s("M5 7.5H9M15 7.5H19"),
                    b("M5 4H9V7.5H5ZM15 4H19V7.5H15Z"),
                ]
            }
        }
        Glyph::Menu => const { &[s("M4 6.5H20M4 12H20M4 17.5H20")] },
        Glyph::Sliders => {
            const {
                &[
                    s("M4 7H6.7M11.3 7H20M4 17H12.7M17.3 17H20"),
                    b("O9 7 2.3"),
                    b("O15 17 2.3"),
                ]
            }
        }
    }
}

// ---------------------------------------------------------------- paths --

type P = [f32; 2];

/// One flattened subpath on the 24-unit grid.
#[derive(Debug)]
struct Sub {
    points: Vec<P>,
    closed: bool,
}

#[derive(Debug, PartialEq)]
enum Token {
    Command(char),
    Number(f32),
}

fn tokens(d: &str) -> Vec<Token> {
    let bytes = d.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            out.push(Token::Command(c));
            i += 1;
        } else if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() {
            let start = i;
            i += 1;
            let mut dot = c == '.';
            while i < bytes.len() {
                let n = bytes[i] as char;
                if n.is_ascii_digit() {
                    i += 1;
                } else if n == '.' && !dot {
                    dot = true;
                    i += 1;
                } else if (n == 'e' || n == 'E')
                    && bytes
                        .get(i + 1)
                        .is_some_and(|&x| x.is_ascii_digit() || x == b'-' || x == b'+')
                {
                    i += 2;
                    while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            out.push(Token::Number(d[start..i].parse().unwrap_or(0.0)));
        } else {
            i += 1;
        }
    }
    out
}

fn arity(command: char) -> usize {
    match command.to_ascii_uppercase() {
        'M' | 'L' | 'T' => 2,
        'H' | 'V' => 1,
        'C' => 6,
        'S' | 'Q' => 4,
        'A' => 7,
        'O' => 3,
        _ => 0,
    }
}

fn cubic(out: &mut Vec<P>, p0: P, p1: P, p2: P, p3: P) {
    const STEPS: usize = 16;
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        out.push([
            w[0] * p0[0] + w[1] * p1[0] + w[2] * p2[0] + w[3] * p3[0],
            w[0] * p0[1] + w[1] * p1[1] + w[2] * p2[1] + w[3] * p3[1],
        ]);
    }
}

fn quad(out: &mut Vec<P>, p0: P, p1: P, p2: P) {
    const STEPS: usize = 12;
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let u = 1.0 - t;
        out.push([
            u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0],
            u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1],
        ]);
    }
}

/// An SVG elliptical arc, converted from endpoints to a center and flattened.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<P>, p0: P, rx: f32, ry: f32, rotation: f32, large: bool, sweep: bool, p1: P) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-6 || ry < 1e-6 || (p0[0] == p1[0] && p0[1] == p1[1]) {
        out.push(p1);
        return;
    }
    let (sin, cos) = rotation.to_radians().sin_cos();
    let (dx, dy) = ((p0[0] - p1[0]) / 2.0, (p0[1] - p1[1]) / 2.0);
    let x1 = cos * dx + sin * dy;
    let y1 = -sin * dx + cos * dy;
    let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let coef = (num / den).max(0.0).sqrt() * if large == sweep { -1.0 } else { 1.0 };
    let cx1 = coef * rx * y1 / ry;
    let cy1 = -coef * ry * x1 / rx;
    let cx = cos * cx1 - sin * cy1 + (p0[0] + p1[0]) / 2.0;
    let cy = sin * cx1 + cos * cy1 + (p0[1] + p1[1]) / 2.0;
    let angle = |u: P, v: P| (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1]);
    let start_v = [(x1 - cx1) / rx, (y1 - cy1) / ry];
    let end_v = [(-x1 - cx1) / rx, (-y1 - cy1) / ry];
    let theta = angle([1.0, 0.0], start_v);
    let mut delta = angle(start_v, end_v);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let steps = ((delta.abs() / (std::f32::consts::PI / 12.0)).ceil() as usize).max(2);
    for i in 1..=steps {
        let t = theta + delta * i as f32 / steps as f32;
        let (st, ct) = t.sin_cos();
        out.push([
            cx + rx * ct * cos - ry * st * sin,
            cy + rx * ct * sin + ry * st * cos,
        ]);
    }
    if let Some(last) = out.last_mut() {
        *last = p1;
    }
}

/// Parse and flatten SVG path data.
fn parse(d: &str) -> Vec<Sub> {
    let tokens = tokens(d);
    let mut subs: Vec<Sub> = Vec::new();
    let mut current: Vec<P> = Vec::new();
    let mut pos: P = [0.0; 2];
    let mut start: P = [0.0; 2];
    // The previous cubic control point, for S.
    let mut last_control: Option<P> = None;
    let mut command = 'M';
    let mut i = 0;
    let finish = |current: &mut Vec<P>, subs: &mut Vec<Sub>, closed: bool| {
        if current.len() > 1 {
            subs.push(Sub {
                points: std::mem::take(current),
                closed,
            });
        } else {
            current.clear();
        }
    };
    while i < tokens.len() {
        if let Token::Command(c) = tokens[i] {
            command = c;
            i += 1;
            if c == 'Z' || c == 'z' {
                finish(&mut current, &mut subs, true);
                pos = start;
                last_control = None;
                continue;
            }
        }
        let n = arity(command);
        if n == 0 || i + n > tokens.len() {
            i += 1;
            continue;
        }
        let mut v = [0.0f32; 7];
        let mut ok = true;
        for (j, slot) in v.iter_mut().enumerate().take(n) {
            match tokens[i + j] {
                Token::Number(x) => *slot = x,
                Token::Command(_) => ok = false,
            }
        }
        if !ok {
            i += 1;
            continue;
        }
        i += n;
        let relative = command.is_ascii_lowercase();
        let at = |x: f32, y: f32| {
            if relative {
                [pos[0] + x, pos[1] + y]
            } else {
                [x, y]
            }
        };
        let mut control = None;
        match command.to_ascii_uppercase() {
            'M' => {
                finish(&mut current, &mut subs, false);
                pos = at(v[0], v[1]);
                start = pos;
                current.push(pos);
                // Further pairs are lines.
                command = if relative { 'l' } else { 'L' };
            }
            'L' | 'T' => {
                pos = at(v[0], v[1]);
                current.push(pos);
            }
            'H' => {
                pos[0] = if relative { pos[0] + v[0] } else { v[0] };
                current.push(pos);
            }
            'V' => {
                pos[1] = if relative { pos[1] + v[0] } else { v[0] };
                current.push(pos);
            }
            'C' => {
                let (c1, c2, end) = (at(v[0], v[1]), at(v[2], v[3]), at(v[4], v[5]));
                cubic(&mut current, pos, c1, c2, end);
                control = Some(c2);
                pos = end;
            }
            'S' => {
                let c1 = last_control.map_or(pos, |c| [2.0 * pos[0] - c[0], 2.0 * pos[1] - c[1]]);
                let (c2, end) = (at(v[0], v[1]), at(v[2], v[3]));
                cubic(&mut current, pos, c1, c2, end);
                control = Some(c2);
                pos = end;
            }
            'Q' => {
                let (c1, end) = (at(v[0], v[1]), at(v[2], v[3]));
                quad(&mut current, pos, c1, end);
                pos = end;
            }
            'A' => {
                let end = at(v[5], v[6]);
                arc(
                    &mut current,
                    pos,
                    v[0],
                    v[1],
                    v[2],
                    v[3] != 0.0,
                    v[4] != 0.0,
                    end,
                );
                pos = end;
            }
            'O' => {
                finish(&mut current, &mut subs, false);
                let (cx, cy, r) = (v[0], v[1], v[2].abs());
                let steps = 32;
                let points = (0..steps)
                    .map(|j| {
                        let a = j as f32 * std::f32::consts::TAU / steps as f32;
                        [cx + r * a.cos(), cy + r * a.sin()]
                    })
                    .collect();
                subs.push(Sub {
                    points,
                    closed: true,
                });
            }
            _ => {}
        }
        if current.is_empty() && !command.eq_ignore_ascii_case(&'O') {
            current.push(pos);
        }
        last_control = control;
    }
    finish(&mut current, &mut subs, false);
    subs
}

// ------------------------------------------------------------ outlines --

fn signed_area(poly: &[P]) -> f32 {
    let mut area = 0.0;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        area += a[0] * b[1] - b[0] * a[1];
    }
    area / 2.0
}

/// Polygons are given one winding so overlapping pieces add up instead of
/// cancelling; coverage is clamped afterwards, which makes their union.
fn push_wound(out: &mut Vec<Vec<P>>, mut poly: Vec<P>) {
    if signed_area(&poly) < 0.0 {
        poly.reverse();
    }
    out.push(poly);
}

fn disc(center: P, r: f32) -> Vec<P> {
    let steps = ((r * 6.0).ceil() as usize).clamp(8, 24);
    (0..steps)
        .map(|j| {
            let a = j as f32 * std::f32::consts::TAU / steps as f32;
            [center[0] + r * a.cos(), center[1] + r * a.sin()]
        })
        .collect()
}

/// Split a polyline into dashes.
fn dashes(points: &[P], closed: bool, on: f32, off: f32) -> Vec<Vec<P>> {
    let mut path = points.to_vec();
    if closed && let Some(&first) = points.first() {
        path.push(first);
    }
    let mut out = Vec::new();
    let mut dash = vec![path[0]];
    let (mut drawing, mut left) = (true, on);
    for w in path.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        while length > 1e-6 {
            let step = left.min(length);
            let t = step / length;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if drawing {
                dash.push(p);
            }
            left -= step;
            length -= step;
            a = p;
            if left <= 1e-6 {
                if drawing && dash.len() > 1 {
                    out.push(std::mem::take(&mut dash));
                }
                drawing = !drawing;
                left = if drawing { on } else { off };
                dash = vec![a];
            }
        }
    }
    if drawing && dash.len() > 1 {
        out.push(dash);
    }
    out
}

/// Outline a line `width` wide as polygons: one quad per segment plus joins
/// and ends, round or square-ended with mitred corners.
fn stroke(points: &[P], closed: bool, width: f32, round: bool, out: &mut Vec<Vec<P>>) {
    let hw = width / 2.0;
    let mut pts: Vec<P> = Vec::with_capacity(points.len());
    for &p in points {
        if pts
            .last()
            .is_none_or(|q: &P| (q[0] - p[0]).abs() + (q[1] - p[1]).abs() > 1e-4)
        {
            pts.push(p);
        }
    }
    if closed && pts.len() > 2 {
        let (first, last) = (pts[0], pts[pts.len() - 1]);
        if (first[0] - last[0]).abs() + (first[1] - last[1]).abs() < 1e-4 {
            pts.pop();
        }
    }
    if pts.len() < 2 {
        if let Some(&p) = pts.first() {
            push_wound(out, disc(p, hw));
        }
        return;
    }
    let count = if closed { pts.len() } else { pts.len() - 1 };
    let direction = |i: usize| {
        let (a, b) = (pts[i % pts.len()], pts[(i + 1) % pts.len()]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
        [dx / l, dy / l]
    };
    for i in 0..count {
        let d = direction(i);
        let n = [-d[1] * hw, d[0] * hw];
        let mut a = pts[i % pts.len()];
        let mut b = pts[(i + 1) % pts.len()];
        if !closed && !round {
            if i == 0 {
                a = [a[0] - d[0] * hw, a[1] - d[1] * hw];
            }
            if i == count - 1 {
                b = [b[0] + d[0] * hw, b[1] + d[1] * hw];
            }
        }
        push_wound(
            out,
            vec![
                [a[0] + n[0], a[1] + n[1]],
                [b[0] + n[0], b[1] + n[1]],
                [b[0] - n[0], b[1] - n[1]],
                [a[0] - n[0], a[1] - n[1]],
            ],
        );
    }
    let joins: Vec<usize> = if closed {
        (0..pts.len()).collect()
    } else {
        (1..pts.len() - 1).collect()
    };
    for j in joins {
        let v = pts[j];
        if round {
            push_wound(out, disc(v, hw));
            continue;
        }
        let d0 = direction((j + pts.len() - 1) % pts.len());
        let d1 = direction(j);
        let cross = d0[0] * d1[1] - d0[1] * d1[0];
        if cross.abs() < 1e-4 {
            continue;
        }
        let sign = if cross > 0.0 { -1.0 } else { 1.0 };
        let o0 = [-d0[1] * sign, d0[0] * sign];
        let o1 = [-d1[1] * sign, d1[0] * sign];
        let dot = o0[0] * o1[0] + o0[1] * o1[1];
        let m = [(o0[0] + o1[0]) / (1.0 + dot), (o0[1] + o1[1]) / (1.0 + dot)];
        let a = [v[0] + o0[0] * hw, v[1] + o0[1] * hw];
        let c = [v[0] + o1[0] * hw, v[1] + o1[1] * hw];
        if (m[0] * m[0] + m[1] * m[1]).sqrt() > 4.0 || dot <= -0.999 {
            push_wound(out, vec![v, a, c]);
        } else {
            push_wound(out, vec![v, a, [v[0] + m[0] * hw, v[1] + m[1] * hw], c]);
        }
    }
    if round && !closed {
        push_wound(out, disc(pts[0], hw));
        push_wound(out, disc(pts[pts.len() - 1], hw));
    }
}

// -------------------------------------------------------------- pixels --

/// Coverage of polygons on a `px`-square grid, each pixel 0 to 1.
fn cover(polys: &[Vec<P>], px: usize) -> Vec<f32> {
    let scale = px as f32 / 24.0;
    let mut raster = Rasterizer::new(px, px);
    for poly in polys {
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            raster.draw_line(
                point(a[0] * scale, a[1] * scale),
                point(b[0] * scale, b[1] * scale),
            );
        }
    }
    let mut out = vec![0.0; px * px];
    raster.for_each_pixel(|index, alpha| {
        if let Some(o) = out.get_mut(index) {
            *o = alpha.min(1.0);
        }
    });
    out
}

/// The icon's alpha at `px` pixels square in `look`.
pub fn rasterize(glyph: Glyph, look: IconLook, px: usize) -> Vec<u8> {
    let look = if look == IconLook::Classic {
        IconLook::Line
    } else {
        look
    };
    let solid = look == IconLook::Solid;
    let round = look != IconLook::Line;
    let line = match look {
        IconLook::Line | IconLook::Classic => 1.5,
        IconLook::Duotone => 2.0,
        IconLook::Solid => 2.1,
    };
    let mut alpha = vec![0.0f32; px * px];
    let mut holes = vec![0.0f32; px * px];
    let lines = |subs: &[Sub], width: f32, dash: Option<[f32; 2]>| {
        let mut polys = Vec::new();
        for sub in subs {
            match dash {
                Some([on, off]) => {
                    for piece in dashes(&sub.points, sub.closed, on, off) {
                        stroke(&piece, false, width, round, &mut polys);
                    }
                }
                None => stroke(&sub.points, sub.closed, width, round, &mut polys),
            }
        }
        cover(&polys, px)
    };
    let area = |subs: &[Sub]| {
        let polys: Vec<Vec<P>> = subs.iter().map(|s| s.points.clone()).collect();
        cover(&polys, px)
    };
    let add = |into: &mut Vec<f32>, coverage: Vec<f32>, weight: f32| {
        for (o, c) in into.iter_mut().zip(coverage) {
            *o = o.max(c * weight);
        }
    };
    for p in parts(glyph) {
        let shown = match p.only {
            Only::All => true,
            Only::Outline => !solid,
            Only::Solid => solid,
        };
        if !shown {
            continue;
        }
        let subs = parse(p.path);
        match (p.role, solid) {
            (Role::Stroke, _) => add(&mut alpha, lines(&subs, line, p.dash), p.opacity),
            (Role::Fill, _) => add(&mut alpha, area(&subs), p.opacity),
            (Role::Body, true) => {
                add(&mut alpha, area(&subs), p.opacity);
                add(&mut alpha, lines(&subs, 1.2, None), p.opacity);
            }
            (Role::Body, false) => {
                if look == IconLook::Duotone {
                    add(&mut alpha, area(&subs), 0.24 * p.opacity);
                }
                add(&mut alpha, lines(&subs, line, p.dash), p.opacity);
            }
            (Role::Detail, true) => add(&mut holes, lines(&subs, 1.7, None), 1.0),
            (Role::Detail, false) => add(&mut alpha, lines(&subs, line, None), p.opacity),
            (Role::DetailFill, true) => add(&mut holes, area(&subs), 1.0),
            (Role::DetailFill, false) => add(&mut alpha, area(&subs), p.opacity),
        }
    }
    alpha
        .into_iter()
        .zip(holes)
        .map(|(a, h)| (a * (1.0 - h) * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect()
}

// ------------------------------------------------------------- drawing --

/// Textures by icon, look and pixel size.
type Textures = HashMap<(Glyph, IconLook, u32), egui::TextureHandle>;

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<Textures>>);

fn texture(ctx: &egui::Context, glyph: Glyph, look: IconLook, px: u32) -> egui::TextureHandle {
    let cache = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Cache>(egui::Id::new("dudel icon art"))
            .clone()
    });
    let mut map = cache.0.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = map.get(&(glyph, look, px)) {
        return handle.clone();
    }
    // Sizes change with display scale and density; a bounded cache keeps a
    // long session from collecting every size ever shown.
    if map.len() > 1500 {
        map.clear();
    }
    let alpha = rasterize(glyph, look, px as usize);
    let image = egui::ColorImage::new(
        [px as usize, px as usize],
        alpha.into_iter().map(Color32::from_white_alpha).collect(),
    );
    let handle = ctx.load_texture(
        format!("icon {glyph:?} {look:?} {px}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    map.insert((glyph, look, px), handle.clone());
    handle
}

/// Draw `glyph` centered in `rect`, pixel-aligned, in `color`.
pub fn paint(painter: &egui::Painter, rect: Rect, glyph: Glyph, look: IconLook, color: Color32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let side = rect.width().min(rect.height());
    let px = (side * ppp).round().clamp(6.0, 256.0) as u32;
    let size = px as f32 / ppp;
    let corner = rect.center() - vec2(size, size) / 2.0;
    let corner = Pos2::new(
        (corner.x * ppp).round() / ppp,
        (corner.y * ppp).round() / ppp,
    );
    let handle = texture(ctx, glyph, look, px);
    painter.image(
        handle.id(),
        Rect::from_min_size(corner, vec2(size, size)),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_commands_tokenize_like_svg() {
        assert_eq!(
            tokens("M1.5-2.5.5.5L3e1 4z"),
            vec![
                Token::Command('M'),
                Token::Number(1.5),
                Token::Number(-2.5),
                Token::Number(0.5),
                Token::Number(0.5),
                Token::Command('L'),
                Token::Number(30.0),
                Token::Number(4.0),
                Token::Command('z'),
            ]
        );
    }

    #[test]
    fn paths_flatten_with_relative_moves_and_arcs() {
        let subs = parse("M4 16h4v4H4zM8 12h4v4H8z");
        assert_eq!(subs.len(), 2);
        assert_eq!(
            subs[0].points,
            vec![[4.0, 16.0], [8.0, 16.0], [8.0, 20.0], [4.0, 20.0]]
        );
        assert!(subs.iter().all(|s| s.closed));
        // A half circle from (2,12) to (22,12) through the bottom (sweep 0
        // goes counterclockwise on screen, so it passes y = 22).
        let half = parse("M2 12A10 10 0 0 0 22 12");
        let lowest = half[0].points.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert!((lowest - 22.0).abs() < 0.2, "{lowest}");
        let end = half[0].points.last().unwrap();
        assert!((end[0] - 22.0).abs() < 1e-3 && (end[1] - 12.0).abs() < 1e-3);
        let circle = parse("O12 12 3");
        assert!(circle[0].closed && circle[0].points.len() == 32);
    }

    #[test]
    fn every_icon_draws_something_inside_its_square_in_every_look() {
        for glyph in Glyph::ALL {
            for look in [IconLook::Line, IconLook::Duotone, IconLook::Solid] {
                let px = 24;
                let alpha = rasterize(glyph, look, px);
                let ink: u32 = alpha.iter().map(|&a| a as u32).sum();
                assert!(ink > 255 * 6, "{glyph:?} {look:?} is nearly empty ({ink})");
                // Nothing fills the whole square except intended solid shapes.
                assert!(ink < 255 * (px * px) as u32 * 9 / 10, "{glyph:?} {look:?}");
            }
        }
    }

    #[test]
    fn solid_cuts_details_out_and_duotone_tints_bodies() {
        let px = 48;
        let at = |alpha: &[u8], x: f32, y: f32| {
            let s = px as f32 / 24.0;
            alpha[(y * s) as usize * px + (x * s) as usize]
        };
        // The eye's pupil: drawn in Line, a hole in the filled Solid eye.
        let line = rasterize(Glyph::Eye, IconLook::Line, px);
        let solid = rasterize(Glyph::Eye, IconLook::Solid, px);
        assert!(at(&line, 12.0, 12.0) > 200);
        assert!(at(&solid, 12.0, 12.0) < 40);
        assert!(at(&solid, 6.0, 12.0) > 200);
        // Inside the lasso loop: empty in Line, tinted in Duotone, full in Solid.
        let inside = |look| at(&rasterize(Glyph::Lasso, look, px), 12.0, 9.5);
        assert!(inside(IconLook::Line) < 10);
        assert!((40..90).contains(&inside(IconLook::Duotone)));
        assert!(inside(IconLook::Solid) > 230);
    }

    #[test]
    fn overlapping_stroke_pieces_do_not_cancel() {
        // A closed square drawn as quads and mitred corners has no holes
        // where the pieces overlap.
        let alpha = rasterize(Glyph::Select, IconLook::Line, 48);
        assert!(alpha.iter().any(|&a| a > 200));
        let frame = rasterize(Glyph::Gradient, IconLook::Line, 48);
        // The top-left corner of the frame at (3.5, 5.5) is covered.
        let s = 2.0;
        assert!(frame[(5.5 * s) as usize * 48 + (3.5 * s) as usize] > 150);
    }
}
