//! One dab onto the active layer's rows: `Document::paint_dab_rows`.
//!
//! What a dab delivers at a pixel is one function, `Coverage::at`. What is
//! done with it is one function for each way of painting:
//!
//! - `prefix_coverage_rows`: a stroke buffer's first pass with a plain round
//!   tip, which only records coverage, a whole row at a time;
//! - `blend_rows`: smudge, blur and wet dabs, which read what is there;
//! - `paint_rows`: plain paint, erasing, mask painting and every other
//!   first pass.
//!
//! The last two hand a large dab on a layer's tiles to `dab_bands`, which
//! does the same arithmetic in row bands across cores.

use crate::brush::Brush;
use crate::dab_bands::Drawn;
use crate::document::{DabRaster, Document, Layer, WATER_LIFT, paint_pigment, paint_pixel};
use crate::material::Kind;
use crate::paint_target::Target;
use crate::stroke_buffer::StrokeBuffer;

/// What a dab delivers at a pixel: the tip's shape, its grain, the flow
/// compensation, the paper and the selection.
struct Coverage<'a> {
    raster: &'a DabRaster,
    brush: &'a Brush,
    /// The paper this dab feels, if it feels one.
    paper: Option<crate::paper::Surface>,
    paper_response: f32,
    selection_mask: Option<&'a crate::selection_mask::Mask>,
}

impl Coverage<'_> {
    /// The dab's alpha and coverage ceiling at a pixel, if it paints there.
    #[inline]
    fn at(&self, px: i32, py: i32) -> Option<(f32, f32)> {
        let (raster, brush) = (self.raster, self.brush);
        let [x, y] = raster.position;
        let (radius, opacity, map) = (raster.radius, raster.opacity, raster.map);
        let (sin, cos) = (raster.sin, raster.cos);
        let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
        let (dx, dy) = (map[0] * dx + map[1] * dy, map[2] * dx + map[3] * dy);
        let u = (dx * cos + dy * sin) / radius;
        let v = (-dx * sin + dy * cos) / (radius * brush.aspect);
        if brush.tip == crate::brush::Tip::Image && u.abs().max(v.abs()) >= 1.0 {
            return None;
        }
        let coverage = if brush.tip == crate::brush::Tip::Image {
            brush
                .image
                .as_ref()
                .map_or(0.0, |image| image.coverage(u, v))
        } else if radius <= 1.0 {
            let mut sum = 0.0;
            for sy in 0..8 {
                for sx in 0..8 {
                    let ox = (sx as f32 + 0.5) / 8.0 - 0.5;
                    let oy = (sy as f32 + 0.5) / 8.0 - 0.5;
                    let mx = map[0] * ox + map[1] * oy;
                    let my = map[2] * ox + map[3] * oy;
                    let su = u + (mx * cos + my * sin) / radius;
                    let sv = v + (-mx * sin + my * cos) / (radius * brush.aspect);
                    let d = if brush.tip == crate::brush::Tip::Round {
                        (su * su + sv * sv).sqrt()
                    } else {
                        su.abs().max(sv.abs())
                    };
                    if d < 1.0 {
                        sum += crate::brush_edge::interior(
                            d,
                            brush.hardness,
                            brush.softness_profile,
                            0.001,
                        );
                    }
                }
            }
            sum / 64.0
        } else {
            crate::brush_edge::coverage_with_profile(
                u,
                v,
                radius,
                brush.aspect,
                brush.hardness,
                brush.tip,
                brush.softness_profile,
            )
        };
        if coverage <= 0.0 {
            return None;
        }
        let mut a = opacity * coverage * brush.grain_coverage(px as usize, py as usize);
        if let Some(ratio) = raster.interval_ratio {
            a = brush.opacity
                * raster.flow_lookup.as_ref().map_or_else(
                    || crate::brush_edge::compensated_delivery(a / brush.opacity, ratio),
                    |lookup| lookup.evaluate(a / opacity),
                );
        }
        if let Some(surface) = &self.paper
            && (self.paper_response > 0.0 || raster.granulation > 0.0)
        {
            // Once per pixel for both.
            let height = surface.height(px as usize, py as usize);
            let paper = &surface.paper;
            if self.paper_response > 0.0 {
                a *= paper.hold_at(
                    height,
                    a / brush.opacity.max(f32::MIN_POSITIVE),
                    self.paper_response,
                );
            }
            if raster.granulation > 0.0 {
                a = (a * paper.pool_at(height, raster.granulation)).min(brush.opacity);
            }
        }
        // After the paper, so each of the dabs this one stands for
        // would have met the same tooth.
        if let Some(ratio) = raster.held {
            a = brush.opacity * crate::brush_edge::compensated_delivery(a / brush.opacity, ratio);
        }
        if a <= 0.0 {
            return None;
        }
        let selected = self
            .selection_mask
            .map_or(1.0, |m| f32::from(m.at(px as usize, py as usize)) / 255.0);
        if selected == 0.0 {
            return None;
        }
        Some((a * selected, brush.opacity * selected))
    }
}

/// A dab's pass over its rows, once the layer and the selection allow it:
/// what each way of painting needs to know.
#[derive(Clone, Copy)]
struct Pass<'a> {
    raster: &'a DabRaster,
    brush: &'a Brush,
    target: Target,
    /// Index of the layer painted.
    active: usize,
    /// The canvas's width and height.
    size: [usize; 2],
    /// The pixels to visit: left, top, right and bottom, each included.
    rect: [i32; 4],
    /// The first row asked for, before clipping: a dab painted in slices
    /// copies what a smudge reads once, on its first slice.
    first_row: i32,
    color: [u8; 3],
    erase: bool,
    alpha_locked: bool,
    kind: Option<Kind>,
    /// Watercolor and thick paint mix as pigments where paint meets paint.
    pigment: bool,
    /// A wet brush, not erasing.
    wet: bool,
    /// A brush graph gives this dab its own color, mixed into the stroke
    /// pixel by pixel.
    tracking: bool,
    /// A stroke buffer's first pass: record coverage and write nothing.
    prefix_pass: bool,
}

impl Document {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_dab_rows(
        &mut self,
        raster: &DabRaster,
        brush: &Brush,
        color: [u8; 3],
        erase: bool,
        buffer: Option<&mut StrokeBuffer>,
        first_row: i32,
        last_row: i32,
    ) {
        let paper = self.paper;
        let [x, y] = raster.position;
        let radius = raster.radius;
        let interval_ratio = raster.interval_ratio;
        let target = buffer
            .as_ref()
            .map_or(crate::paint_target::Target::Raster, |b| b.target);
        let active = self.active;
        let layer = &mut self.layers[self.active];
        if !buffer.as_ref().map_or_else(
            || target.valid(layer, [self.width, self.height]),
            |b| b.target_valid(layer, [self.width, self.height]),
        ) || layer.locked
            || !layer.visible
            || (target.is_raster() && erase && layer.alpha_locked)
        {
            return;
        }
        // Materials change how paint lands on raster pixels (`material`).
        let material = layer
            .material
            .as_ref()
            .map(|m| m.settings)
            .filter(|_| target.is_raster());
        let kind = material.map(|m| m.kind);
        // Dry media always feels a tooth: the canvas's paper, or its own.
        let dry_tooth = material
            .filter(|m| m.kind == crate::material::Kind::Dry)
            .map(|m| m.tooth);
        let paper = match dry_tooth {
            Some(tooth) => Some(crate::material::dry_paper(paper, tooth)),
            None => paper,
        };
        let paper_response = dry_tooth.map_or(raster.paper_response, |tooth| {
            raster.paper_response.max(tooth.sqrt())
        });
        // Every other case takes the general path below, selections and
        // tips of a pixel or less among them.
        if radius > 1.0
            && brush.tip == crate::brush::Tip::Round
            && brush.grain.is_none()
            && brush.grain_amount == 0.0
            && interval_ratio.is_none()
            && self.selection.is_none()
            && self.selection_mask.is_none()
            && paper.is_none()
            && kind != Some(crate::material::Kind::Paint)
            && raster.right >= raster.left
            && raster.right - raster.left < 512
            && buffer.as_ref().is_some_and(|b| b.is_prefix_pass())
        {
            prefix_coverage_rows(raster, brush, active, buffer.unwrap(), first_row, last_row);
            return;
        }
        // The eraser end still erases with a smudge or blur brush.
        let blending = !brush.blend.is_paint() && !erase;
        // Watercolor and thick paint mix as pigments where paint meets paint.
        let pigment = matches!(
            kind,
            Some(crate::material::Kind::Watercolor | crate::material::Kind::Paint)
        );
        let wet = brush.blend == crate::brush::Blend::Wet && !erase;
        // Paper tooth holds back paint laid on artwork, and granulating
        // paint settles in its valleys; erasing, smudging and mask painting
        // ignore the paper.
        let paper = paper
            .filter(|_| target.is_raster() && !erase && !brush.blend.moves_paint())
            .map(|paper| paper.surface());
        // A graph that sets color gives this dab its own color, mixed into
        // the stroke pixel by pixel.
        let tracking = raster.color_shift.is_some() && !erase;
        let color = raster
            .color_shift
            .map_or(color, |shift| shift.apply(color, raster.secondary));
        let (color, erase, alpha_locked) = target.params(color, erase, layer.alpha_locked);
        let prefix_pass = buffer.as_ref().is_some_and(|b| b.is_prefix_pass());
        // Clip the iteration domain before evaluating coverage or touching COW
        // storage. Work outside a rectangular selection cannot affect artwork.
        let (mut left, mut right) = (raster.left, raster.right);
        let (mut top, mut bottom) = (first_row.max(raster.top), last_row.min(raster.bottom));
        if let Some(selection) = self.selection {
            left = left.max(selection.left as i32);
            right = right.min(selection.right as i32 - 1);
            top = top.max(selection.top as i32);
            bottom = bottom.min(selection.bottom as i32 - 1);
        }
        if left > right || top > bottom {
            return;
        }
        let coverage = Coverage {
            raster,
            brush,
            paper,
            paper_response,
            selection_mask: self.selection_mask.as_ref(),
        };
        let coverage_at = |px: i32, py: i32| coverage.at(px, py);
        // Smudge, blur, wet and thick paint dabs go pixel by pixel on this
        // thread, and thick paint asks for each pixel's coverage twice (the
        // relief, then the paint). For a large dab the coverage, the costly
        // part, is worked out once here, in bands across cores.
        let (span, rows) = ((right - left + 1) as usize, (bottom - top + 1) as usize);
        let blending_now = !brush.blend.is_paint() && !erase && !prefix_pass;
        let thick = kind == Some(crate::material::Kind::Paint) && !prefix_pass;
        let area = [
            left as usize,
            top as usize,
            right as usize + 1,
            bottom as usize + 1,
        ];
        let drawn =
            ((blending_now || thick) && crate::dab_bands::worthwhile(span, rows)).then(|| {
                let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Coverage);
                crate::dab_bands::Drawn::new(area, |px, py| coverage_at(px as i32, py as i32))
            });
        let coverage_at = |px: i32, py: i32| -> Option<(f32, f32)> {
            match &drawn {
                Some(drawn) => drawn.at(px as usize, py as usize),
                None => coverage_at(px, py),
            }
        };
        // Thick paint: the dab builds, wears or carries the relief where it
        // paints (once per dab; a prefix pass repeats dabs already painted).
        if kind == Some(crate::material::Kind::Paint)
            && !prefix_pass
            && let Some(material) = layer.material.as_mut()
        {
            let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Relief);
            crate::material::touch_relief(
                material,
                [self.width, self.height],
                &crate::material::Touch {
                    rect: [left, top, right, bottom],
                    center: [x, y],
                    radius,
                    // Streaks follow the stroke once it has a direction.
                    heading: raster.heading.map(|h| h.to_radians().sin_cos()),
                    smear: raster.smear,
                    blend: brush.blend,
                    thickness: raster.thickness,
                    erase,
                },
                &coverage_at,
            );
        }
        let pass = Pass {
            raster,
            brush,
            target,
            active,
            size: [self.width, self.height],
            rect: [left, top, right, bottom],
            first_row,
            color,
            erase,
            alpha_locked,
            kind,
            pigment,
            wet,
            tracking,
            prefix_pass,
        };
        if blending && !prefix_pass {
            blend_rows(
                &mut self.wet,
                layer,
                &pass,
                &coverage_at,
                drawn.as_ref(),
                buffer,
            );
        } else {
            paint_rows(layer, &pass, &coverage_at, drawn.as_ref(), buffer);
        }
    }
}

/// A stroke buffer's first pass for a plain round tip with nothing else to
/// consider (no grain, paper, selection or thick paint): the coverage of
/// whole rows, kept apart from the stores that depend on one another.
fn prefix_coverage_rows(
    raster: &DabRaster,
    brush: &Brush,
    active: usize,
    buffer: &mut StrokeBuffer,
    first_row: i32,
    last_row: i32,
) {
    let [x, y] = raster.position;
    let (radius, opacity, map) = (raster.radius, raster.opacity, raster.map);
    let (sin, cos) = (raster.sin, raster.cos);
    let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Prefix);
    let mut delivery = [0.0f32; 512];
    let width = (raster.right - raster.left + 1) as usize;
    for py in first_row.max(raster.top)..=last_row.min(raster.bottom) {
        for (offset, value) in delivery[..width].iter_mut().enumerate() {
            let px = raster.left + offset as i32;
            let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
            let (dx, dy) = (map[0] * dx + map[1] * dy, map[2] * dx + map[3] * dy);
            let u = (dx * cos + dy * sin) / radius;
            let v = (-dx * sin + dy * cos) / (radius * brush.aspect);
            let coverage = crate::brush_edge::coverage_with_profile(
                u,
                v,
                radius,
                brush.aspect,
                brush.hardness,
                brush.tip,
                brush.softness_profile,
            );
            *value = if coverage <= 0.0 {
                0.0
            } else {
                opacity * coverage
            };
        }
        buffer.commit_coverage_row(
            active,
            raster.left as usize,
            py as usize,
            &delivery[..width],
            brush.opacity,
        );
    }
}

/// Smudge, blur and wet dabs: each pixel is mixed with what the dab read
/// from the layer, and a wet dab leaves water in `field`.
fn blend_rows(
    field: &mut crate::wet::Field,
    layer: &mut Layer,
    pass: &Pass,
    coverage_at: &(impl Fn(i32, i32) -> Option<(f32, f32)> + Sync),
    drawn: Option<&Drawn>,
    mut buffer: Option<&mut StrokeBuffer>,
) {
    let Pass {
        raster,
        brush,
        target,
        active,
        first_row,
        color,
        alpha_locked,
        kind,
        pigment,
        wet,
        ..
    } = *pass;
    let [left, top, right, bottom] = pass.rect;
    let (width, radius) = (pass.size[0], raster.radius);
    let rows = (bottom - top + 1) as usize;
    let mut scratch = buffer
        .as_deref_mut()
        .map(|b| std::mem::take(&mut b.blend_scratch))
        .unwrap_or_default();
    // Rows of one dab may arrive in slices; the copy is made once.
    if first_row <= raster.top {
        crate::blend_brush::capture(
            &mut scratch,
            |px, py| target.read(layer, width, px, py),
            pass.size,
            [raster.left, raster.top, raster.right, raster.bottom],
            brush.blend,
            raster.smear,
            radius,
        );
    }
    let layer_id = layer.id;
    // Dry media stays dry.
    let watering = wet && target.is_raster() && kind != Some(crate::material::Kind::Dry);
    let span = (right - left + 1) as usize;
    // Water already here, and water this dab leaves, per row.
    let (mut was_wet, mut water) = if wet {
        (vec![0.0f32; span], vec![0.0f32; span])
    } else {
        (Vec::new(), Vec::new())
    };
    // A large dab on a layer's tiles blends in row bands across
    // cores, with the arithmetic of the loop below.
    if let Some(drawn) = drawn
        && target.is_raster()
        && layer.pixels.is_tiled()
        && let Some(buffer) = buffer.as_deref_mut()
    {
        let mut damp = Vec::new();
        if watering {
            let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::WaterRead);
            damp = vec![0.0f32; span * rows];
            for (row, values) in damp.chunks_exact_mut(span).enumerate() {
                field.read_row(layer_id, top as usize + row, left as usize, values);
            }
        }
        let (left, top) = (left as usize, top as usize);
        let load = raster.pigment;
        let timed = crate::stage_timer::scope(crate::stage_timer::Stage::Blend);
        let left_water = crate::dab_bands::apply(
            &mut layer.pixels,
            buffer,
            active,
            drawn,
            watering,
            |px, py, a, ceiling, entry, pixel| {
                if entry.1 < 0.0 {
                    *entry = ((&*pixel).try_into().unwrap(), 0.0);
                }
                let source = scratch.source(brush.blend, raster.smear, px, py);
                if !wet {
                    crate::blend_brush::mix(pixel, source, a, alpha_locked, pigment);
                    return 0.0;
                }
                let strength = a / ceiling.max(f32::MIN_POSITIVE);
                let damp = damp
                    .get((py - top) * span + (px - left))
                    .copied()
                    .unwrap_or(0.0);
                if damp > 0.0 {
                    crate::blend_brush::mix(
                        pixel,
                        source,
                        (damp * strength).min(1.0) * 0.7,
                        alpha_locked,
                        pigment,
                    );
                }
                if load < 1.0 && !alpha_locked {
                    let lift = strength.min(1.0) * (1.0 - load) * WATER_LIFT;
                    pixel[3] = (f32::from(pixel[3]) * (1.0 - lift)).round() as u8;
                }
                if pigment && damp > 0.0 && pixel[3] > 0 {
                    paint_pigment(pixel, a * load, color, alpha_locked);
                } else {
                    paint_pixel(pixel, a * load, color, false, alpha_locked);
                }
                strength.min(1.0) * raster.wetness
            },
        );
        drop(timed);
        let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::WaterWrite);
        let rows = left_water.iter().flat_map(|band| band.chunks_exact(span));
        for (row, water) in rows.enumerate() {
            field.add_row(layer_id, top + row, left, water);
        }
        buffer.blend_scratch = scratch;
        return;
    }
    let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Serial);
    let mut writer = target.writer(layer, width);
    for py in top..=bottom {
        if watering {
            let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::SerialWater);
            field.read_row(layer_id, py as usize, left as usize, &mut was_wet);
            water.fill(0.0);
        }
        for px in left..=right {
            let Some((a, ceiling)) = coverage_at(px, py) else {
                continue;
            };
            let column = (px - left) as usize;
            let (px, py) = (px as usize, py as usize);
            let source = scratch.source(brush.blend, raster.smear, px, py);
            writer.edit(px, py, |pixel| {
                if let Some(buffer) = buffer.as_deref_mut() {
                    buffer.remember_at(active, px, py, pixel);
                }
                if wet {
                    // Wet-on-wet: paint still wet here runs together
                    // before the new paint lands. Dry paint stays put.
                    let strength = a / ceiling.max(f32::MIN_POSITIVE);
                    let damp = was_wet.get(column).copied().unwrap_or(0.0);
                    if damp > 0.0 {
                        crate::blend_brush::mix(
                            pixel,
                            source,
                            (damp * strength).min(1.0) * 0.7,
                            alpha_locked,
                            pigment,
                        );
                    }
                    // Clear water lifts a little of what is there.
                    let load = raster.pigment;
                    if load < 1.0 && !alpha_locked {
                        let lift = strength.min(1.0) * (1.0 - load) * WATER_LIFT;
                        pixel[3] = (f32::from(pixel[3]) * (1.0 - lift)).round() as u8;
                    }
                    if pigment && damp > 0.0 && pixel[3] > 0 {
                        paint_pigment(pixel, a * load, color, alpha_locked);
                    } else {
                        paint_pixel(pixel, a * load, color, false, alpha_locked);
                    }
                    if watering {
                        water[column] = strength.min(1.0) * raster.wetness;
                    }
                } else {
                    crate::blend_brush::mix(pixel, source, a, alpha_locked, pigment);
                }
            });
        }
        if watering {
            let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::SerialWater);
            field.add_row(layer_id, py as usize, left as usize, &water);
        }
    }
    if let Some(buffer) = buffer {
        buffer.blend_scratch = scratch;
    }
}

/// Plain paint, erasing and mask painting, and the first pass of a stroke
/// buffer for every tip `prefix_coverage_rows` does not take.
fn paint_rows(
    layer: &mut Layer,
    pass: &Pass,
    coverage_at: &(impl Fn(i32, i32) -> Option<(f32, f32)> + Sync),
    drawn: Option<&Drawn>,
    mut buffer: Option<&mut StrokeBuffer>,
) {
    let Pass {
        target,
        active,
        color,
        erase,
        alpha_locked,
        tracking,
        prefix_pass,
        ..
    } = *pass;
    let [left, top, right, bottom] = pass.rect;
    let area = [
        left as usize,
        top as usize,
        right as usize + 1,
        bottom as usize + 1,
    ];
    let _timed = crate::stage_timer::scope(if prefix_pass {
        crate::stage_timer::Stage::Prefix
    } else {
        crate::stage_timer::Stage::Paint
    });
    // Large dabs on raster tiles blend in row bands across cores.
    if !prefix_pass
        && !tracking
        && target.is_raster()
        && layer.pixels.is_tiled()
        && let Some(buffer) = buffer.as_deref_mut()
        && crate::dab_bands::worthwhile((right - left + 1) as usize, (bottom - top + 1) as usize)
    {
        let made;
        let drawn = match drawn {
            Some(drawn) => drawn,
            None => {
                made =
                    crate::dab_bands::Drawn::new(area, |px, py| coverage_at(px as i32, py as i32));
                &made
            }
        };
        crate::dab_bands::paint(
            &mut layer.pixels,
            buffer,
            active,
            drawn,
            color,
            erase,
            alpha_locked,
        );
        return;
    }
    let mut writer = (!prefix_pass).then(|| target.writer(layer, pass.size[0]));
    let scalar_writes = writer
        .as_ref()
        .is_some_and(|writer| writer.prefers_scalar_dabs((right - left + 1) as usize));
    // Fixed scratch keeps each resumable row bounded without allocating.
    // Separate coverage from writes so tile ownership is checked per span,
    // while retaining pixel order and the existing blending arithmetic.
    let mut delivery = [0.0f32; 1024];
    for py in top..=bottom {
        for chunk_left in (left..=right).step_by(512) {
            let chunk_right = right.min(chunk_left + 511);
            let count = (chunk_right - chunk_left + 1) as usize;
            if !prefix_pass && !scalar_writes {
                delivery[..count * 2].fill(0.0);
            }
            for px in chunk_left..=chunk_right {
                let Some((mut a, ceiling)) = coverage_at(px, py) else {
                    continue;
                };
                let (px, py) = (px as usize, py as usize);
                if prefix_pass {
                    if let Some(buffer) = buffer.as_deref_mut() {
                        buffer.commit_coverage_at(active, px, py, a, ceiling);
                    }
                    continue;
                }
                if scalar_writes {
                    writer.as_mut().unwrap().edit(px, py, |pixels| {
                        let mut color = color;
                        if let Some(buffer) = buffer.as_deref_mut() {
                            if tracking {
                                (a, color) = buffer
                                    .accumulate_color_at(active, px, py, pixels, a, ceiling, color);
                            } else {
                                a = buffer.accumulate_at(active, px, py, pixels, a, ceiling);
                            }
                        }
                        paint_pixel(pixels, a, color, erase, alpha_locked);
                    });
                } else {
                    delivery[(px - chunk_left as usize) * 2] = a;
                    delivery[(px - chunk_left as usize) * 2 + 1] = ceiling;
                }
            }
            if prefix_pass || scalar_writes {
                continue;
            }
            let mut at = 0;
            while at < count {
                if delivery[at * 2] <= 0.0 {
                    at += 1;
                    continue;
                }
                let start = at;
                while at < count && delivery[at * 2] > 0.0 {
                    at += 1;
                }
                writer.as_mut().unwrap().edit_span(
                    chunk_left as usize + start,
                    py as usize,
                    at - start,
                    |offset, pixels| {
                        let index = start + offset;
                        let mut a = delivery[index * 2];
                        let mut color = color;
                        if let Some(buffer) = buffer.as_deref_mut() {
                            let (x, ceiling) =
                                (chunk_left as usize + index, delivery[index * 2 + 1]);
                            if tracking {
                                (a, color) = buffer.accumulate_color_at(
                                    active,
                                    x,
                                    py as usize,
                                    pixels,
                                    a,
                                    ceiling,
                                    color,
                                );
                            } else {
                                a = buffer.accumulate_at(
                                    active,
                                    x,
                                    py as usize,
                                    pixels,
                                    a,
                                    ceiling,
                                );
                            }
                        }
                        paint_pixel(pixels, a, color, erase, alpha_locked);
                    },
                );
            }
        }
    }
}
