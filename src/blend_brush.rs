//! App side of `blend_brush`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::blend_brush`, re-exported
//! here so `crate::blend_brush::…` paths keep working.
pub(crate) use dudel_core::blend_brush::*;

impl crate::Dudel {
    /// Screenshot scene: color bands with a smudge wave and a blur stroke
    /// across them, and the Smudge preset in the tool panel.
    pub(crate) fn smudge_capture_fixture(&mut self) {
        self.tool_panel_override = Some(true);
        self.select_tool(crate::tools::Tool::Brush);
        if let Some(index) = self.brushes.iter().position(|b| b.name == "Smudge") {
            self.brush = self.brushes[index].clone();
            self.selected_brush = index;
        }
        let (width, height) = (self.doc.width, self.doc.height);
        let bands = [[226, 94, 62], [246, 196, 70], [70, 150, 120], [60, 92, 170]];
        {
            let mut writer = self.doc.layers[self.doc.active].pixels.writer();
            for y in 0..height {
                for x in 0..width {
                    let c = bands[x / 48 % bands.len()];
                    writer
                        .rgba_mut(y * width + x)
                        .copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
        let run =
            |doc: &mut crate::document::Document, brush: crate::brush::Brush, y: f32, wave: f32| {
                let settings = crate::stroke_buffer::Settings {
                    target: crate::paint_target::Target::Raster,
                    brush,
                    color: [0, 0, 0],
                    secondary: [0, 0, 0],
                    erase: false,
                    layer: doc.active,
                    symmetry: Default::default(),
                };
                let mut stroke = crate::stroke::Stroke::default();
                let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
                let steps = 120;
                for i in 0..=steps {
                    let t = i as f32 / steps as f32;
                    let sample = crate::stroke::Sample {
                        x: width as f32 * (0.08 + 0.84 * t),
                        y: y + (t * std::f32::consts::TAU * 1.5).sin() * wave,
                        pressure: 0.9,
                        pressure_known: true,
                        ..Default::default()
                    };
                    let dabs = if i == steps {
                        stroke.finish(sample, &settings.brush)
                    } else {
                        stroke.sample(sample, &settings.brush)
                    };
                    for dab in dabs {
                        buffer.paint(doc, dab, &settings);
                    }
                }
            };
        let smudge = self.brush.clone();
        run(
            &mut self.doc,
            smudge,
            height as f32 * 0.3,
            height as f32 * 0.08,
        );
        if let Some(blur) = self.brushes.iter().find(|b| b.name == "Blur").cloned() {
            run(&mut self.doc, blur, height as f32 * 0.7, 0.0);
        }
        self.changed();
    }
}
