//! Shown vector layers as an SVG: every line a path. Lines of one width are
//! stroked center lines; lines shaped by pressure or taper are outlines.
//! Raster layers, masks, effects and brush texture are not vector artwork
//! and are left out.
use crate::{Dudel, document::Document};
use std::fmt::Write;

/// Widths closer than this (as a share of the widest) make a constant line.
const EVEN_WIDTH: f32 = 0.05;

fn number(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn point(x: f32, y: f32) -> String {
    format!("{} {}", number(x), number(y))
}

/// `run` with points closer than a pixel to the last kept one left out.
fn thinned(run: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut out: Vec<[f32; 3]> = Vec::with_capacity(run.len());
    for &p in run {
        if out
            .last()
            .is_none_or(|q| (p[0] - q[0]).hypot(p[1] - q[1]) >= 1.0)
        {
            out.push(p);
        }
    }
    if let (Some(&last), true) = (run.last(), out.len() > 1) {
        *out.last_mut().unwrap() = last;
    }
    out
}

/// The line's outline: down one side, round its end, back up the other and
/// round its start.
fn outline(run: &[[f32; 3]]) -> String {
    let side = |i: usize| {
        let (a, b) = (run[i.saturating_sub(1)], run[(i + 1).min(run.len() - 1)]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = dx.hypot(dy).max(1e-3);
        [dy / length * run[i][2], -dx / length * run[i][2]]
    };
    let cap = |radius: f32, x: f32, y: f32| {
        let r = number(radius);
        format!("A{r} {r} 0 0 1 {}", point(x, y))
    };
    let mut path = String::new();
    for (i, p) in run.iter().enumerate() {
        let [ox, oy] = side(i);
        let verb = if i == 0 { 'M' } else { 'L' };
        let _ = write!(path, "{verb}{} ", point(p[0] + ox, p[1] + oy));
    }
    let last = run.len() - 1;
    let [ox, oy] = side(last);
    let _ = write!(
        path,
        "{} ",
        cap(run[last][2], run[last][0] - ox, run[last][1] - oy)
    );
    for i in (0..last).rev() {
        let [ox, oy] = side(i);
        let _ = write!(path, "L{} ", point(run[i][0] - ox, run[i][1] - oy));
    }
    let [ox, oy] = side(0);
    path + cap(run[0][2], run[0][0] + ox, run[0][1] + oy).as_str() + "Z"
}

fn write_run(out: &mut String, run: &[[f32; 3]], paint: &str, opacity: &str) {
    let run = thinned(run);
    let Some(&[x, y, radius]) = run.first() else {
        return;
    };
    if run.len() == 1 {
        let _ = writeln!(
            out,
            r#"<circle cx="{}" cy="{}" r="{}" fill="{paint}"{opacity}/>"#,
            number(x),
            number(y),
            number(radius)
        );
        return;
    }
    let (low, high) = run.iter().fold((f32::MAX, 0.0f32), |(low, high), p| {
        (low.min(p[2]), high.max(p[2]))
    });
    if high - low <= high * EVEN_WIDTH {
        let mut d = format!("M{}", point(x, y));
        for p in &run[1..] {
            let _ = write!(d, " L{}", point(p[0], p[1]));
        }
        let _ = writeln!(
            out,
            r#"<path d="{d}" fill="none" stroke="{paint}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"{opacity}/>"#,
            number(low + high)
        );
    } else {
        let _ = writeln!(
            out,
            r#"<path d="{}" fill="{paint}"{opacity}/>"#,
            outline(&run)
        );
    }
}

/// The document's shown vector layers as SVG text, and how many lines it holds.
pub(crate) fn encode(doc: &Document) -> (String, usize) {
    let size = [doc.width, doc.height];
    let (width, height) = (doc.width, doc.height);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">\n"
    );
    let mut lines = 0;
    for (index, layer) in doc.layers.iter().enumerate() {
        let Some(vector) = layer.vector.as_ref().filter(|_| doc.shown(index)) else {
            continue;
        };
        let _ = writeln!(
            out,
            r#"<g data-name="{}" opacity="{}">"#,
            escape(&layer.name),
            number(layer.opacity)
        );
        for stroke in &vector.strokes {
            let [r, g, b] = stroke.color;
            let paint = format!("#{r:02x}{g:02x}{b:02x}");
            let opacity = if stroke.csp.is_none() && stroke.brush.opacity < 1.0 {
                format!(r#" opacity="{}""#, number(stroke.brush.opacity))
            } else {
                String::new()
            };
            for run in stroke.runs(size) {
                write_run(&mut out, &run, &paint, &opacity);
            }
            lines += 1;
        }
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    (out, lines)
}

impl Dudel {
    pub(crate) fn export_svg(&mut self) {
        let name = self
            .path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map_or("Painting".into(), |s| s.to_string_lossy().into_owned());
        let dialog = self.dialog_in(
            crate::locations::Kind::Exports,
            rfd::AsyncFileDialog::new()
                .add_filter("SVG", &["svg"])
                .set_file_name(format!("{name}.svg")),
        );
        self.ask_save_file(dialog, move |app, path| app.export_svg_to(&path));
    }
    pub(crate) fn export_svg_to(&mut self, path: &std::path::Path) {
        self.flush_render_queue();
        let (svg, lines) = encode(&self.doc);
        self.status = match crate::storage::atomic_write(path, svg.as_bytes()) {
            Ok(()) => format!("Exported {lines} vector line(s) to SVG; raster layers are left out"),
            Err(error) => error,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_go_round_a_line_clockwise_on_screen() {
        // Rightward on screen: the first side is above the line, and the
        // end cap sweeps round past its far end.
        let path = outline(&[[0.0, 10.0, 2.0], [10.0, 10.0, 2.0]]);
        assert_eq!(path, "M0 8 L10 8 A2 2 0 0 1 10 12 L0 12 A2 2 0 0 1 0 8Z");
    }

    #[test]
    fn close_points_thin_out_but_the_ends_stay() {
        let run = [
            [0.0, 0.0, 1.0],
            [0.2, 0.0, 1.0],
            [1.5, 0.0, 1.0],
            [1.6, 0.0, 1.0],
        ];
        let thin = thinned(&run);
        assert_eq!(thin.len(), 2);
        assert_eq!(thin[1], run[3]);
    }

    #[test]
    fn numbers_stay_short_and_names_stay_safe() {
        assert_eq!(number(12.5), "12.5");
        assert_eq!(number(3.0), "3");
        assert_eq!(number(0.0), "0");
        assert_eq!(escape(r#"a<b>&"c""#), "a&lt;b&gt;&amp;&quot;c&quot;");
    }
}
