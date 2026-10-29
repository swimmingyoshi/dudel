//! Photoshop vector content: action descriptors (fill and stroke settings)
//! and vector mask paths, rasterized to coverage. Layouts follow Adobe's
//! Photoshop File Formats Specification; path-operation semantics that the
//! specification leaves out follow the independent psd-tools reader.

/// Big-endian reader over an untrusted slice.
struct Bytes<'a>(&'a [u8]);
impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.0.len() {
            return Err("Truncated PSD descriptor".into());
        }
        let (data, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(data)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn count(&mut self, limit: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n > limit {
            return Err("PSD descriptor count exceeds limits".into());
        }
        Ok(n)
    }
    /// A key or class ID: a length, or 0 for a four-byte code.
    fn key(&mut self) -> Result<String, String> {
        let n = self.count(1024)?;
        Ok(String::from_utf8_lossy(self.take(if n == 0 { 4 } else { n })?).into_owned())
    }
    fn unicode(&mut self) -> Result<String, String> {
        let n = self.count(1 << 16)?;
        let units: Vec<u16> = self
            .take(n * 2)?
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&units)
            .trim_end_matches('\0')
            .to_owned())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Number(f64),
    /// A unit-carrying number, such as `#Pxl` pixels or `#Prc` percent.
    Unit(String, f64),
    Text(String),
    /// Enumeration type and value.
    Enum(String, String),
    Integer(i64),
    Bool(bool),
    Object(Descriptor),
    List(Vec<Value>),
    Class(String),
    Raw,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Descriptor {
    pub class: String,
    pub items: Vec<(String, Value)>,
}
impl Descriptor {
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub(crate) fn number(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Value::Number(v) | Value::Unit(_, v) => Some(*v),
            Value::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }
    pub(crate) fn unit(&self, key: &str) -> Option<(&str, f64)> {
        match self.get(key)? {
            Value::Unit(unit, v) => Some((unit, *v)),
            Value::Number(v) => Some(("", *v)),
            _ => None,
        }
    }
    pub(crate) fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }
    pub(crate) fn object(&self, key: &str) -> Option<&Descriptor> {
        match self.get(key)? {
            Value::Object(d) => Some(d),
            _ => None,
        }
    }
    pub(crate) fn enumerated(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Enum(_, v) => Some(v),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 32;
const MAX_ITEMS: usize = 4096;

/// A descriptor preceded by its 16 version word.
pub(crate) fn versioned(data: &[u8]) -> Result<Descriptor, String> {
    let mut bytes = Bytes(data);
    if bytes.u32()? != 16 {
        return Err("Unsupported PSD descriptor version".into());
    }
    descriptor(&mut bytes, 0)
}
/// `vscg`: a fill key and version, then a descriptor. Returns the key.
pub(crate) fn keyed(data: &[u8]) -> Result<(String, Descriptor), String> {
    let mut bytes = Bytes(data);
    let key = String::from_utf8_lossy(bytes.take(4)?).into_owned();
    bytes.u32()?;
    Ok((key, descriptor(&mut bytes, 0)?))
}
fn descriptor(bytes: &mut Bytes<'_>, depth: usize) -> Result<Descriptor, String> {
    if depth > MAX_DEPTH {
        return Err("PSD descriptor nesting exceeds limits".into());
    }
    bytes.unicode()?;
    let class = bytes.key()?;
    let count = bytes.count(MAX_ITEMS)?;
    let mut items = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        let key = bytes.key()?;
        let value = value(bytes, depth)?;
        items.push((key, value));
    }
    Ok(Descriptor { class, items })
}
fn value(bytes: &mut Bytes<'_>, depth: usize) -> Result<Value, String> {
    let kind: [u8; 4] = bytes.take(4)?.try_into().unwrap();
    Ok(match &kind {
        b"Objc" | b"GlbO" => Value::Object(descriptor(bytes, depth + 1)?),
        b"VlLs" => {
            let n = bytes.count(MAX_ITEMS)?;
            let mut list = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                list.push(value(bytes, depth + 1)?);
            }
            Value::List(list)
        }
        b"doub" => Value::Number(bytes.f64()?),
        b"UntF" => {
            let unit = String::from_utf8_lossy(bytes.take(4)?).into_owned();
            Value::Unit(unit, bytes.f64()?)
        }
        b"TEXT" => Value::Text(bytes.unicode()?),
        b"enum" => Value::Enum(bytes.key()?, bytes.key()?),
        b"long" => Value::Integer(i64::from(bytes.u32()? as i32)),
        b"comp" => Value::Integer(i64::from_be_bytes(bytes.take(8)?.try_into().unwrap())),
        b"bool" => Value::Bool(bytes.take(1)?[0] != 0),
        b"type" | b"GlbC" => {
            bytes.unicode()?;
            Value::Class(bytes.key()?)
        }
        b"tdta" | b"alis" | b"Pth " => {
            let n = bytes.count(64 << 20)?;
            bytes.take(n)?;
            Value::Raw
        }
        b"UnFl" => {
            bytes.take(4)?;
            let n = bytes.count(MAX_ITEMS)?;
            bytes.take(n * 8)?;
            Value::Raw
        }
        _ => {
            return Err(format!(
                "Unsupported PSD descriptor value {}",
                String::from_utf8_lossy(&kind)
            ));
        }
    })
}

/// An RGB color from a descriptor color object (RGB or grayscale).
pub(crate) fn color(object: &Descriptor) -> Result<[u8; 3], String> {
    let channel = |key: &str| {
        object
            .number(key)
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("PSD color lacks {key}"))
    };
    let rgb = match object.class.as_str() {
        "RGBC" => [channel("Rd  ")?, channel("Grn ")?, channel("Bl  ")?],
        "HSBC" => {
            let (h, s, v) = (
                channel("H   ")?.rem_euclid(360.0) / 60.0,
                channel("Strt")? / 100.0,
                channel("Brgh")? / 100.0 * 255.0,
            );
            let f = h.fract();
            let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
            match h as u32 {
                0 => [v, t, p],
                1 => [q, v, p],
                2 => [p, v, t],
                3 => [p, q, v],
                4 => [t, p, v],
                _ => [v, p, q],
            }
        }
        // Gray is a percentage of black ink.
        "Grsc" => [255.0 * (1.0 - channel("Gry ")? / 100.0); 3],
        other => return Err(format!("Unsupported PSD color model {other}")),
    };
    Ok(rgb.map(|v| v.round().clamp(0.0, 255.0) as u8))
}
/// The solid color of a fill descriptor (`SoCo`, or `vscg` with key SoCo).
pub(crate) fn solid_fill(fill: &Descriptor) -> Result<[u8; 3], String> {
    color(fill.object("Clr ").ok_or("PSD solid fill has no color")?)
}

/// A gradient fill (`GdFl`, or `vscg` with key GdFl).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Gradient {
    kind: GradientKind,
    angle: f64,
    scale: f64,
    reverse: bool,
    /// Lay the ramp out over the layer (true) or the whole document.
    aligned: bool,
    offset: [f64; 2],
    /// (location 0..=1, color, midpoint 0..=1 of the segment ending here).
    colors: Vec<(f64, [f64; 3], f64)>,
    opacities: Vec<(f64, f64, f64)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum GradientKind {
    Linear,
    Radial,
    Angle,
    Reflected,
    Diamond,
}
pub(crate) fn gradient(settings: &Descriptor) -> Result<Gradient, String> {
    let ramp = settings.object("Grad").ok_or("PSD gradient has no ramp")?;
    if ramp.enumerated("GrdF") != Some("CstS") {
        return Err("Noise PSD gradients need the saved composite".into());
    }
    let kind = match settings.enumerated("Type") {
        Some("Lnr ") | None => GradientKind::Linear,
        Some("Rdl ") => GradientKind::Radial,
        Some("Angl") => GradientKind::Angle,
        Some("Rflc") => GradientKind::Reflected,
        Some("Dmnd") => GradientKind::Diamond,
        Some(other) => return Err(format!("Unsupported PSD gradient type {other}")),
    };
    let stops = |key: &str| -> Result<Vec<&Descriptor>, String> {
        match ramp.get(key) {
            Some(Value::List(items)) => items
                .iter()
                .map(|item| match item {
                    Value::Object(stop) => Ok(stop),
                    _ => Err("Invalid PSD gradient stop".to_string()),
                })
                .collect(),
            None => Ok(Vec::new()),
            _ => Err("Invalid PSD gradient stops".into()),
        }
    };
    let place = |stop: &Descriptor| {
        (
            (stop.number("Lctn").unwrap_or(0.0) / 4096.0).clamp(0.0, 1.0),
            (stop.number("Mdpn").unwrap_or(50.0) / 100.0).clamp(0.05, 0.95),
        )
    };
    let mut colors = Vec::new();
    for stop in stops("Clrs")? {
        // Foreground/background stops depend on the editor's colors.
        if !matches!(stop.enumerated("Type"), None | Some("UsrS")) {
            return Err("PSD gradient stops using editor colors need the saved composite".into());
        }
        let color = color(
            stop.object("Clr ")
                .ok_or("PSD gradient stop has no color")?,
        )?;
        let (location, midpoint) = place(stop);
        colors.push((location, color.map(f64::from), midpoint));
    }
    let mut opacities = Vec::new();
    for stop in stops("Trns")? {
        let (location, midpoint) = place(stop);
        let opacity = (stop.number("Opct").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);
        opacities.push((location, opacity, midpoint));
    }
    if colors.is_empty() || colors.len() > 1024 || opacities.len() > 1024 {
        return Err("PSD gradient stops exceed limits".into());
    }
    if opacities.is_empty() {
        opacities.push((0.0, 1.0, 0.5));
    }
    let offset = settings.object("Ofst").map_or([0.0; 2], |point| {
        [
            point.number("Hrzn").unwrap_or(0.0) / 100.0,
            point.number("Vrtc").unwrap_or(0.0) / 100.0,
        ]
    });
    let finite = |v: f64| v.is_finite().then_some(v);
    Ok(Gradient {
        kind,
        angle: settings.number("Angl").and_then(finite).unwrap_or(0.0),
        scale: (settings.number("Scl ").and_then(finite).unwrap_or(100.0) / 100.0)
            .clamp(0.01, 100.0),
        reverse: settings.flag("Rvrs").unwrap_or(false),
        aligned: settings.flag("Algn").unwrap_or(true),
        offset: offset.map(|v| if v.is_finite() { v } else { 0.0 }),
        colors,
        opacities,
    })
}
/// Interpolate stops at `t`, honoring each segment's midpoint.
fn ramp<const N: usize>(stops: &[(f64, [f64; N], f64)], t: f64) -> [f64; N] {
    let first = stops[0];
    if t <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t <= b.0 {
            let span = b.0 - a.0;
            let mut u = if span > 0.0 { (t - a.0) / span } else { 1.0 };
            if (b.2 - 0.5).abs() > 1e-6 {
                u = u.powf(0.5f64.ln() / b.2.ln());
            }
            return std::array::from_fn(|c| a.1[c] + (b.1[c] - a.1[c]) * u);
        }
    }
    stops[stops.len() - 1].1
}
impl Gradient {
    /// Straight RGBA at pixel (x, y). `layer` is the box the ramp spans when
    /// aligned with the layer; otherwise the document is used.
    pub(crate) fn painter(
        &self,
        layer: [f64; 4],
        document: [usize; 2],
    ) -> impl Fn(usize, usize) -> [u8; 4] + '_ {
        let [left, top, right, bottom] = if self.aligned {
            layer
        } else {
            [0.0, 0.0, document[0] as f64, document[1] as f64]
        };
        let (w, h) = ((right - left).max(1.0), (bottom - top).max(1.0));
        // The ramp spans the box's width at 0° and its height at 90°.
        let turn = self.angle.rem_euclid(180.0);
        let ratio = if turn <= 90.0 { turn } else { 180.0 - turn };
        let reach = self.scale * ((90.0 - ratio) / 90.0 * w + ratio / 90.0 * h);
        let center = [
            left + w / 2.0 + self.offset[0] * w,
            top + h / 2.0 + self.offset[1] * h,
        ];
        let theta = self.angle.to_radians();
        let (sin, cos) = theta.sin_cos();
        let colors: Vec<_> = self.colors.clone();
        let opacities: Vec<(f64, [f64; 1], f64)> = self
            .opacities
            .iter()
            .map(|&(l, o, m)| (l, [o], m))
            .collect();
        move |x, y| {
            // Ramp coordinates: -1..1 across the box (scaled).
            let gx = 2.0 * (x as f64 + 0.5 - center[0]) / reach;
            let gy = 2.0 * (y as f64 + 0.5 - center[1]) / reach;
            let mut t = match self.kind {
                GradientKind::Linear => 0.5 * (cos * gx - sin * gy + 1.0),
                GradientKind::Radial => (gx * gx + gy * gy).sqrt(),
                GradientKind::Angle => {
                    ((gy.atan2(gx).to_degrees() + self.angle).rem_euclid(360.0)) / 360.0
                }
                GradientKind::Reflected => (cos * gx - sin * gy).abs(),
                GradientKind::Diamond => (cos * gx - sin * gy).abs() + (sin * gx + cos * gy).abs(),
            }
            .clamp(0.0, 1.0);
            if self.reverse {
                t = 1.0 - t;
            }
            let rgb = ramp(&colors, t);
            let alpha = ramp(&opacities, t)[0];
            [
                rgb[0].round().clamp(0.0, 255.0) as u8,
                rgb[1].round().clamp(0.0, 255.0) as u8,
                rgb[2].round().clamp(0.0, 255.0) as u8,
                (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
            ]
        }
    }
}

/// A vector shape's outline settings from `vstk`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stroke {
    pub width: f32,
    pub align: Align,
    pub color: [u8; 3],
    pub opacity: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Align {
    Inside,
    Center,
    Outside,
}
/// What `vstk` asks for: whether the shape is filled and its stroke, if any.
pub(crate) fn stroke(settings: &Descriptor) -> Result<(bool, Option<Stroke>), String> {
    let fill = settings.flag("fillEnabled").unwrap_or(true);
    if !settings.flag("strokeEnabled").unwrap_or(false) {
        return Ok((fill, None));
    }
    let dashed = match settings.get("strokeStyleLineDashSet") {
        Some(Value::List(dashes)) => !dashes.is_empty(),
        _ => false,
    };
    if dashed {
        return Err("Dashed PSD shape strokes need the saved composite".into());
    }
    let (unit, width) = settings
        .unit("strokeStyleLineWidth")
        .ok_or("PSD stroke has no width")?;
    if !matches!(unit, "#Pxl" | "") || !(0.0..=1000.0).contains(&width) {
        return Err("Unsupported PSD stroke width".into());
    }
    let align = match settings.enumerated("strokeStyleLineAlignment") {
        Some("strokeStyleAlignInside") => Align::Inside,
        Some("strokeStyleAlignOutside") => Align::Outside,
        _ => Align::Center,
    };
    let content = settings
        .object("strokeStyleContent")
        .ok_or("PSD stroke has no content")?;
    if content.class != "solidColorLayer" {
        return Err("Gradient or pattern PSD strokes need the saved composite".into());
    }
    let opacity = settings
        .number("strokeStyleOpacity")
        .map_or(1.0, |v| (v / 100.0).clamp(0.0, 1.0)) as f32;
    Ok((
        fill,
        Some(Stroke {
            width: width as f32,
            align,
            color: solid_fill(content)?,
            opacity,
        }),
    ))
}

/// One knot: incoming control, anchor and outgoing control, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Knot {
    before: [f64; 2],
    anchor: [f64; 2],
    after: [f64; 2],
}
#[derive(Clone, Debug, PartialEq)]
struct Subpath {
    closed: bool,
    /// 0 exclude, 1 union, 2 subtract, 3 intersect; -1 joins the previous
    /// subpath into one component.
    operation: i16,
    /// Non-zero winding instead of the default even-odd rule.
    nonzero: bool,
    knots: Vec<Knot>,
}
/// A vector mask (`vmsk`/`vsms`): flags and subpaths in document pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VectorMask {
    pub inverted: bool,
    pub disabled: bool,
    initial_fill: bool,
    paths: Vec<Subpath>,
}
const MAX_KNOTS: usize = 1 << 16;
pub(crate) fn vector_mask(data: &[u8], width: usize, height: usize) -> Result<VectorMask, String> {
    let mut bytes = Bytes(data);
    if bytes.u32()? != 3 {
        return Err("Unsupported PSD vector mask version".into());
    }
    let flags = bytes.u32()?;
    let mut mask = VectorMask {
        inverted: flags & 1 != 0,
        disabled: flags & 4 != 0,
        initial_fill: false,
        paths: Vec::new(),
    };
    let mut knots = 0;
    while bytes.0.len() >= 26 {
        let mut record = Bytes(bytes.take(26)?);
        let selector = record.u16()?;
        match selector {
            0 | 3 => {
                let length = record.u16()?;
                let operation = record.u16()? as i16;
                let nonzero = record.u16()? == 2;
                mask.paths.push(Subpath {
                    closed: selector == 0,
                    operation,
                    nonzero,
                    knots: Vec::with_capacity(usize::from(length).min(64)),
                });
            }
            1 | 2 | 4 | 5 => {
                knots += 1;
                if knots > MAX_KNOTS {
                    return Err("PSD vector path exceeds limits".into());
                }
                let mut point = || -> Result<[f64; 2], String> {
                    let y = f64::from(record.u32()? as i32) / f64::from(1 << 24);
                    let x = f64::from(record.u32()? as i32) / f64::from(1 << 24);
                    Ok([x * width as f64, y * height as f64])
                };
                let knot = Knot {
                    before: point()?,
                    anchor: point()?,
                    after: point()?,
                };
                mask.paths
                    .last_mut()
                    .ok_or("PSD path knot before its subpath")?
                    .knots
                    .push(knot);
            }
            8 => mask.initial_fill = record.u16()? != 0,
            6 | 7 => {}
            _ => return Err("Unknown PSD path record".into()),
        }
    }
    if bytes.0.iter().any(|b| *b != 0) {
        return Err("Truncated PSD vector path".into());
    }
    Ok(mask)
}

/// Line segments of a subpath, with cubic segments flattened.
fn outline(path: &Subpath) -> Vec<[[f64; 2]; 2]> {
    let n = path.knots.len();
    let mut segments = Vec::new();
    let pairs = if path.closed { n } else { n.saturating_sub(1) };
    for i in 0..pairs {
        let (a, b) = (path.knots[i], path.knots[(i + 1) % n]);
        let p = [a.anchor, a.after, b.before, b.anchor];
        let length: f64 = (0..3)
            .map(|k| ((p[k + 1][0] - p[k][0]).powi(2) + (p[k + 1][1] - p[k][1]).powi(2)).sqrt())
            .sum();
        let steps = (length / 2.0).ceil().clamp(1.0, 256.0) as usize;
        let at = |t: f64| {
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            [0, 1].map(|c| (0..4).map(|k| w[k] * p[k][c]).sum::<f64>())
        };
        let mut previous = p[0];
        for s in 1..=steps {
            let next = at(s as f64 / steps as f64);
            segments.push([previous, next]);
            previous = next;
        }
    }
    segments
}

const SUBROWS: usize = 16;
/// Area coverage of closed polygons, per pixel, in 0..=1, by the even-odd
/// or non-zero winding rule.
fn fill(
    nonzero: bool,
    segments: &[[[f64; 2]; 2]],
    width: usize,
    height: usize,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, String> {
    let mut plane = vec![0f32; width * height];
    if segments.is_empty() {
        return Ok(plane);
    }
    let top = segments
        .iter()
        .flat_map(|s| [s[0][1], s[1][1]])
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as usize;
    let bottom = (segments
        .iter()
        .flat_map(|s| [s[0][1], s[1][1]])
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .max(0.0) as usize)
        .min(height);
    // Segments that reach each row, so a row only tests its own edges.
    let mut starting: Vec<Vec<usize>> = vec![Vec::new(); bottom.saturating_sub(top)];
    for (i, [a, b]) in segments.iter().enumerate() {
        let first = (a[1].min(b[1]).floor().max(top as f64) as usize).min(bottom);
        if first < bottom {
            starting[first - top].push(i);
        }
    }
    let mut active: Vec<usize> = Vec::new();
    let mut crossings = Vec::new();
    for y in top..bottom {
        if cancel() {
            return Err("PSD import canceled".into());
        }
        active.extend(&starting[y - top]);
        active.retain(|&i| segments[i][0][1].max(segments[i][1][1]) >= y as f64);
        let row = &mut plane[y * width..(y + 1) * width];
        for sub in 0..SUBROWS {
            let sy = y as f64 + (sub as f64 + 0.5) / SUBROWS as f64;
            crossings.clear();
            for [a, b] in active.iter().map(|&i| &segments[i]) {
                if (a[1] <= sy) != (b[1] <= sy) {
                    let x = a[0] + (sy - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                    crossings.push((x, if b[1] > a[1] { 1 } else { -1 }));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            // Spans where the rule says the point is inside.
            let mut spans = Vec::new();
            let mut winding = 0i32;
            for &(x, direction) in &crossings {
                let was = if nonzero {
                    winding != 0
                } else {
                    winding % 2 != 0
                };
                winding += if nonzero { direction } else { 1 };
                let now = if nonzero {
                    winding != 0
                } else {
                    winding % 2 != 0
                };
                if !was && now {
                    spans.push([x, x]);
                } else if was && !now {
                    spans.last_mut().unwrap()[1] = x;
                }
            }
            for span in spans {
                let (x0, x1) = (
                    span[0].clamp(0.0, width as f64),
                    span[1].clamp(0.0, width as f64),
                );
                if x1 <= x0 {
                    continue;
                }
                let weight = 1.0 / SUBROWS as f32;
                let (first, last) = (x0.floor() as usize, (x1.ceil() as usize).min(width));
                for (x, cell) in row.iter_mut().enumerate().take(last).skip(first) {
                    let covered = (x1.min(x as f64 + 1.0) - x0.max(x as f64)).max(0.0);
                    *cell += covered as f32 * weight;
                }
            }
        }
    }
    Ok(plane)
}

impl VectorMask {
    /// Bounds of the path's anchors and control points, in pixels.
    pub(crate) fn bounds(&self) -> Option<[f64; 4]> {
        let points = self.paths.iter().flat_map(|p| &p.knots).map(|k| k.anchor);
        points.fold(None, |b: Option<[f64; 4]>, p| {
            Some(b.map_or([p[0], p[1], p[0], p[1]], |b| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            }))
        })
    }
    /// Coverage of the filled path (before `inverted`), one value per pixel.
    pub(crate) fn coverage(
        &self,
        width: usize,
        height: usize,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<f32>, String> {
        let start = if self.initial_fill && self.paths.is_empty() {
            1.0
        } else {
            0.0
        };
        let mut mask = vec![start; width * height];
        // Subpaths with operation -1 join the component before them.
        let mut components: Vec<Vec<&Subpath>> = Vec::new();
        for path in &self.paths {
            match components.last_mut() {
                Some(component) if path.operation == -1 => component.push(path),
                _ => components.push(vec![path]),
            }
        }
        for (index, component) in components.iter().enumerate() {
            let segments: Vec<_> = component
                .iter()
                .filter(|p| p.knots.len() > 1)
                .flat_map(|p| {
                    let mut path = (*p).clone();
                    // Filling treats every subpath as closed.
                    path.closed = true;
                    outline(&path)
                })
                .collect();
            let plane = fill(component[0].nonzero, &segments, width, height, cancel)?;
            let operation = component[0].operation;
            if index == 0 && matches!(operation, 2 | 3) {
                mask.iter_mut().for_each(|m| *m = 1.0 - *m);
            }
            for (m, p) in mask.iter_mut().zip(plane) {
                let p = p.min(1.0);
                *m = match operation {
                    0 => *m + p - 2.0 * *m * p,
                    2 => (*m - p).max(0.0),
                    3 => *m * p,
                    _ => *m + p - *m * p,
                };
            }
        }
        Ok(mask)
    }
    /// Coverage of a stroke of `stroke.width` along the path outline.
    pub(crate) fn stroke_coverage(
        &self,
        stroke: &Stroke,
        filled: &[f32],
        width: usize,
        height: usize,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<f32>, String> {
        // Photoshop strokes the outline of the combined shape, so a closed
        // subpath's edge counts only where it separates inside from outside
        // (pieces are at most a pixel long, so partial edges are kept).
        let inside = |x: f64, y: f64| {
            x >= 0.0
                && y >= 0.0
                && (x as usize) < width
                && (y as usize) < height
                && filled[y as usize * width + x as usize] > 0.5
        };
        let mut segments = Vec::new();
        for path in self.paths.iter().filter(|p| p.knots.len() > 1) {
            for [a, b] in outline(path) {
                let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                let pieces = length.ceil().max(1.0) as usize;
                for k in 0..pieces {
                    let t = |k: usize| k as f64 / pieces as f64;
                    let p0 = [a[0] + (b[0] - a[0]) * t(k), a[1] + (b[1] - a[1]) * t(k)];
                    let p1 = [
                        a[0] + (b[0] - a[0]) * t(k + 1),
                        a[1] + (b[1] - a[1]) * t(k + 1),
                    ];
                    if path.closed && length > 0.0 {
                        let mid = [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0];
                        let normal = [
                            -(b[1] - a[1]) / length * 0.75,
                            (b[0] - a[0]) / length * 0.75,
                        ];
                        if inside(mid[0] + normal[0], mid[1] + normal[1])
                            == inside(mid[0] - normal[0], mid[1] - normal[1])
                        {
                            continue;
                        }
                    }
                    segments.push([p0, p1]);
                }
            }
        }
        let reach = match stroke.align {
            Align::Center => stroke.width as f64 / 2.0,
            _ => stroke.width as f64,
        };
        let mut plane = vec![0f32; width * height];
        if segments.is_empty() || reach <= 0.0 {
            return Ok(plane);
        }
        let pad = reach + 1.0;
        let bounds = segments.iter().flatten().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            },
        );
        let clamp = |v: f64, max: usize| v.max(0.0).min(max as f64) as usize;
        let (x0, y0) = (
            clamp((bounds[0] - pad).floor(), width),
            clamp((bounds[1] - pad).floor(), height),
        );
        let (x1, y1) = (
            clamp((bounds[2] + pad).ceil(), width),
            clamp((bounds[3] + pad).ceil(), height),
        );
        // Each grid cell lists the segments within reach of it.
        let cell = pad.ceil().max(8.0);
        let columns = ((x1 - x0) as f64 / cell).ceil().max(1.0) as usize;
        let rows = ((y1 - y0) as f64 / cell).ceil().max(1.0) as usize;
        let index = |v: f64, origin: usize, count: usize| {
            (((v - origin as f64) / cell).floor().max(0.0) as usize).min(count - 1)
        };
        let mut grid: Vec<Vec<usize>> = vec![Vec::new(); columns * rows];
        for (i, [a, b]) in segments.iter().enumerate() {
            let cx = [a[0].min(b[0]) - pad, a[0].max(b[0]) + pad].map(|v| index(v, x0, columns));
            let cy = [a[1].min(b[1]) - pad, a[1].max(b[1]) + pad].map(|v| index(v, y0, rows));
            for gy in cy[0]..=cy[1] {
                for gx in cx[0]..=cx[1] {
                    grid[gy * columns + gx].push(i);
                }
            }
        }
        for y in y0..y1 {
            if cancel() {
                return Err("PSD import canceled".into());
            }
            for x in x0..x1 {
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                let near = &grid[index(p[1], y0, rows) * columns + index(p[0], x0, columns)];
                let distance = near
                    .iter()
                    .map(|&i| segments[i])
                    .map(|[a, b]| {
                        let d = [b[0] - a[0], b[1] - a[1]];
                        let length = d[0] * d[0] + d[1] * d[1];
                        let t = if length > 0.0 {
                            (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        ((p[0] - a[0] - t * d[0]).powi(2) + (p[1] - a[1] - t * d[1]).powi(2)).sqrt()
                    })
                    .fold(f64::INFINITY, f64::min);
                let band = (reach - distance + 0.5).clamp(0.0, 1.0) as f32;
                let inside = filled[y * width + x];
                plane[y * width + x] = match stroke.align {
                    Align::Center => band,
                    Align::Inside => band * inside,
                    Align::Outside => band * (1.0 - inside),
                };
            }
        }
        Ok(plane)
    }
}

/// A descriptor as bytes (without a version word), for building test files.
#[cfg(test)]
pub(crate) fn encode(descriptor: &Descriptor) -> Vec<u8> {
    fn key(out: &mut Vec<u8>, k: &str) {
        out.extend((if k.len() == 4 { 0 } else { k.len() as u32 }).to_be_bytes());
        out.extend(k.as_bytes());
    }
    fn body(out: &mut Vec<u8>, d: &Descriptor) {
        out.extend(0u32.to_be_bytes());
        key(out, &d.class);
        out.extend((d.items.len() as u32).to_be_bytes());
        for (k, v) in &d.items {
            key(out, k);
            value(out, v);
        }
    }
    fn value(out: &mut Vec<u8>, v: &Value) {
        match v {
            Value::Number(n) => {
                out.extend(b"doub");
                out.extend(n.to_be_bytes());
            }
            Value::Unit(unit, n) => {
                out.extend(b"UntF");
                out.extend(unit.as_bytes());
                out.extend(n.to_be_bytes());
            }
            Value::Text(text) => {
                out.extend(b"TEXT");
                let units: Vec<u16> = text.encode_utf16().collect();
                out.extend((units.len() as u32).to_be_bytes());
                units.iter().for_each(|u| out.extend(u.to_be_bytes()));
            }
            Value::Enum(kind, item) => {
                out.extend(b"enum");
                key(out, kind);
                key(out, item);
            }
            Value::Integer(n) => {
                out.extend(b"long");
                out.extend((*n as i32).to_be_bytes());
            }
            Value::Bool(b) => out.extend([b'b', b'o', b'o', b'l', u8::from(*b)]),
            Value::Object(d) => {
                out.extend(b"Objc");
                body(out, d);
            }
            Value::List(items) => {
                out.extend(b"VlLs");
                out.extend((items.len() as u32).to_be_bytes());
                items.iter().for_each(|item| value(out, item));
            }
            Value::Class(class) => {
                out.extend(b"type");
                out.extend(0u32.to_be_bytes());
                key(out, class);
            }
            Value::Raw => unimplemented!(),
        }
    }
    let mut out = Vec::new();
    body(&mut out, descriptor);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn knot(x: f64, y: f64, w: f64, h: f64) -> Vec<u8> {
        let fixed = |v: f64| ((v * f64::from(1 << 24)) as i32).to_be_bytes();
        let mut out = Vec::new();
        for _ in 0..3 {
            out.extend(fixed(y / h));
            out.extend(fixed(x / w));
        }
        out
    }
    /// A vector mask of closed straight-edged polygons (operation per polygon).
    pub(crate) fn polygon_mask(polygons: &[(i16, &[[f64; 2]])], w: f64, h: f64) -> Vec<u8> {
        let mut data = vec![0, 0, 0, 3, 0, 0, 0, 0];
        let record = |selector: u16, body: &[u8]| {
            let mut r = selector.to_be_bytes().to_vec();
            r.extend(body);
            r.resize(26, 0);
            r
        };
        data.extend(record(6, &[]));
        for (operation, points) in polygons {
            let mut head = (points.len() as u16).to_be_bytes().to_vec();
            head.extend(operation.to_be_bytes());
            data.extend(record(0, &head));
            for [x, y] in *points {
                data.extend(record(2, &knot(*x, *y, w, h)));
            }
        }
        data
    }
    #[test]
    fn rectangles_fill_with_area_coverage_and_operations() {
        let never = || false;
        let square = [[2.0, 2.0], [6.0, 2.0], [6.0, 6.0], [2.0, 6.0]];
        let mask = vector_mask(&polygon_mask(&[(1, &square)], 8.0, 8.0), 8, 8).unwrap();
        let c = mask.coverage(8, 8, &never).unwrap();
        assert_eq!(c[3 * 8 + 3], 1.0);
        assert_eq!(c[8 + 1], 0.0);
        // A half-pixel edge covers half the pixel.
        let half = [[2.5, 2.0], [6.0, 2.0], [6.0, 6.0], [2.5, 6.0]];
        let mask = vector_mask(&polygon_mask(&[(1, &half)], 8.0, 8.0), 8, 8).unwrap();
        assert!((mask.coverage(8, 8, &never).unwrap()[3 * 8 + 2] - 0.5).abs() < 0.01);
        // Subtracting an inner square leaves a ring; -1 merges into an
        // even-odd hole as well.
        let inner = [[3.0, 3.0], [5.0, 3.0], [5.0, 5.0], [3.0, 5.0]];
        for operation in [2, -1] {
            let mask = vector_mask(
                &polygon_mask(&[(1, &square), (operation, &inner)], 8.0, 8.0),
                8,
                8,
            )
            .unwrap();
            let c = mask.coverage(8, 8, &never).unwrap();
            assert_eq!(c[4 * 8 + 4], 0.0, "{operation}");
            assert_eq!(c[2 * 8 + 2], 1.0, "{operation}");
        }
        // Two same-direction squares merged into one component: even-odd
        // cuts the overlap out, non-zero (header flag 2) keeps it.
        let twice = polygon_mask(&[(1, &square), (-1, &square)], 8.0, 8.0);
        let c = vector_mask(&twice, 8, 8)
            .unwrap()
            .coverage(8, 8, &never)
            .unwrap();
        assert_eq!(c[3 * 8 + 3], 0.0);
        let mut nonzero = twice.clone();
        // The two subpath length records are records 1 and 6 (after the
        // fill-rule record and four knots); their flags follow the operation.
        for record in [1, 6] {
            let at = 8 + 26 * record + 6;
            nonzero[at..at + 2].copy_from_slice(&2u16.to_be_bytes());
        }
        let c = vector_mask(&nonzero, 8, 8)
            .unwrap()
            .coverage(8, 8, &never)
            .unwrap();
        assert_eq!(c[3 * 8 + 3], 1.0);
        // Intersect first starts from full coverage.
        let mask = vector_mask(&polygon_mask(&[(3, &square)], 8.0, 8.0), 8, 8).unwrap();
        let c = mask.coverage(8, 8, &never).unwrap();
        assert_eq!((c[0], c[3 * 8 + 3]), (0.0, 1.0));
    }
    #[test]
    fn gradients_follow_type_direction_stops_and_midpoints() {
        let base = Gradient {
            kind: GradientKind::Linear,
            angle: 0.0,
            scale: 1.0,
            reverse: false,
            aligned: true,
            offset: [0.0; 2],
            colors: vec![(0.0, [0.0; 3], 0.5), (1.0, [255.0; 3], 0.5)],
            opacities: vec![(0.0, 1.0, 0.5)],
        };
        let at = |g: &Gradient, x: usize, y: usize| {
            g.painter([0.0, 0.0, 100.0, 100.0], [100, 100])(x, y)
        };
        // Linear at 0 degrees runs left to right across the box.
        assert!(at(&base, 0, 50)[0] < 5 && at(&base, 99, 50)[0] > 250);
        assert!((i32::from(at(&base, 50, 50)[0]) - 128).abs() <= 2);
        // 90 degrees runs bottom to top; reversed swaps the ends.
        let up = Gradient {
            angle: 90.0,
            ..base.clone()
        };
        assert!(at(&up, 50, 99)[0] < 5 && at(&up, 50, 0)[0] > 250);
        let reversed = Gradient {
            reverse: true,
            ..base.clone()
        };
        assert!(at(&reversed, 0, 50)[0] > 250);
        // Radial is darkest at the centre, reflected is symmetric.
        let radial = Gradient {
            kind: GradientKind::Radial,
            ..base.clone()
        };
        assert!(at(&radial, 50, 50)[0] < 10 && at(&radial, 0, 50)[0] > 240);
        let reflected = Gradient {
            kind: GradientKind::Reflected,
            ..base.clone()
        };
        assert_eq!(at(&reflected, 20, 50), at(&reflected, 79, 50));
        // A 25% midpoint reaches the halfway color a quarter of the way along.
        let early = Gradient {
            colors: vec![(0.0, [0.0; 3], 0.5), (1.0, [255.0; 3], 0.25)],
            ..base.clone()
        };
        assert!((i32::from(at(&early, 25, 50)[0]) - 128).abs() <= 4);
        // Opacity stops fade alpha independently of color.
        let fading = Gradient {
            opacities: vec![(0.0, 1.0, 0.5), (1.0, 0.0, 0.5)],
            ..base.clone()
        };
        assert!(at(&fading, 0, 50)[3] > 250 && at(&fading, 99, 50)[3] < 5);
        // Not aligned with the layer: the ramp spans the document instead.
        let document = Gradient {
            aligned: false,
            ..base
        };
        assert!(document.painter([0.0, 0.0, 10.0, 10.0], [100, 100])(50, 0)[0] > 120);
    }
    #[test]
    fn strokes_follow_alignment_and_damaged_paths_fail() {
        let never = || false;
        let square = [[4.0, 4.0], [12.0, 4.0], [12.0, 12.0], [4.0, 12.0]];
        let mask = vector_mask(&polygon_mask(&[(1, &square)], 16.0, 16.0), 16, 16).unwrap();
        let filled = mask.coverage(16, 16, &never).unwrap();
        let stroke = |align| Stroke {
            width: 2.0,
            align,
            color: [0; 3],
            opacity: 1.0,
        };
        let at = |align, x: usize, y: usize| {
            mask.stroke_coverage(&stroke(align), &filled, 16, 16, &never)
                .unwrap()[y * 16 + x]
        };
        // Pixel 5 lies inside the left edge (x = 4), pixel 2 outside it.
        assert_eq!(at(Align::Inside, 5, 8), 1.0);
        assert_eq!(at(Align::Inside, 2, 8), 0.0);
        assert_eq!(at(Align::Outside, 2, 8), 1.0);
        assert_eq!(at(Align::Outside, 5, 8), 0.0);
        assert_eq!(at(Align::Center, 8, 8), 0.0);
        let data = polygon_mask(&[(1, &square)], 16.0, 16.0);
        for end in 0..8 {
            assert!(vector_mask(&data[..end], 16, 16).is_err());
        }
        assert!(vector_mask(&data[..data.len() - 3], 16, 16).is_err());
    }
    #[test]
    fn descriptors_read_colors_strokes_and_reject_damage() {
        fn key(out: &mut Vec<u8>, k: &str) {
            if k.len() == 4 {
                out.extend(0u32.to_be_bytes());
            } else {
                out.extend((k.len() as u32).to_be_bytes());
            }
            out.extend(k.as_bytes());
        }
        fn head(out: &mut Vec<u8>, class: &str, items: u32) {
            out.extend(1u32.to_be_bytes());
            out.extend([0, 0]);
            key(out, class);
            out.extend(items.to_be_bytes());
        }
        let mut color = Vec::new();
        head(&mut color, "RGBC", 3);
        for (k, v) in [("Rd  ", 255.0f64), ("Grn ", 128.0), ("Bl  ", 0.0)] {
            key(&mut color, k);
            color.extend(b"doub");
            color.extend(v.to_be_bytes());
        }
        let mut data = 16u32.to_be_bytes().to_vec();
        head(&mut data, "null", 1);
        key(&mut data, "Clr ");
        data.extend(b"Objc");
        data.extend(&color);
        let fill = versioned(&data).unwrap();
        assert_eq!(solid_fill(&fill).unwrap(), [255, 128, 0]);
        for end in 0..data.len() {
            assert!(versioned(&data[..end]).is_err(), "{end}");
        }
        // Stroke settings: width in pixels, outside alignment, solid content.
        let mut stroke_data = 16u32.to_be_bytes().to_vec();
        head(&mut stroke_data, "strokeStyle", 4);
        key(&mut stroke_data, "strokeEnabled");
        stroke_data.extend(b"bool\x01");
        key(&mut stroke_data, "strokeStyleLineWidth");
        stroke_data.extend(b"UntF#Pxl");
        stroke_data.extend(3.0f64.to_be_bytes());
        key(&mut stroke_data, "strokeStyleLineAlignment");
        stroke_data.extend(b"enum");
        key(&mut stroke_data, "strokeStyleLineAlignment");
        key(&mut stroke_data, "strokeStyleAlignOutside");
        key(&mut stroke_data, "strokeStyleContent");
        stroke_data.extend(b"Objc");
        head(&mut stroke_data, "solidColorLayer", 1);
        key(&mut stroke_data, "Clr ");
        stroke_data.extend(b"Objc");
        stroke_data.extend(&color);
        let (filled, stroke) = stroke(&versioned(&stroke_data).unwrap()).unwrap();
        assert!(filled);
        let stroke = stroke.unwrap();
        assert_eq!(
            (stroke.width, stroke.align, stroke.color),
            (3.0, Align::Outside, [255, 128, 0])
        );
    }
}
