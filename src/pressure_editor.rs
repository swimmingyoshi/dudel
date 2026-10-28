use crate::brush::Brush;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, brush: &mut Brush, pressure: Option<f32>) {
    ui.checkbox(&mut brush.custom_pressure, "Custom pressure curve");
    if !brush.custom_pressure {
        // The long label sits above so the slider row fits narrow panels.
        ui.label("Pressure response");
        ui.add(egui::Slider::new(&mut brush.pressure_gamma, 0.2..=5.0));
        ui.label("1 = linear · lower = lighter touch · higher = firmer touch");
    }
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(320.0), 130.0),
        egui::Sense::hover(),
    );
    let plot = rect.shrink(12.0);
    let point = |x: f32, y: f32| {
        egui::pos2(
            plot.left() + x * plot.width(),
            plot.bottom() - y * plot.height(),
        )
    };
    ui.painter()
        .rect_filled(plot, 4.0, egui::Color32::from_gray(24));
    for i in 0..=4 {
        let t = i as f32 / 4.0;
        ui.painter().line_segment(
            [point(t, 0.0), point(t, 1.0)],
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(48)),
        );
        ui.painter().line_segment(
            [point(0.0, t), point(1.0, t)],
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(48)),
        );
    }
    if brush.custom_pressure {
        for index in 1..4 {
            let handle = point(index as f32 / 4.0, brush.pressure_curve[index]);
            let response = ui.interact(
                egui::Rect::from_center_size(handle, egui::vec2(24.0, 24.0)),
                ui.id().with(("pressure knot", index)),
                egui::Sense::drag(),
            );
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                brush.pressure_curve[index] = ((plot.bottom() - pos.y) / plot.height()).clamp(
                    brush.pressure_curve[index - 1],
                    brush.pressure_curve[index + 1],
                );
            }
            response.on_hover_text(format!(
                "Input {}% · drag vertically to change output",
                index * 25
            ));
        }
    }
    let points = (0..=64)
        .map(|i| {
            let x = i as f32 / 64.0;
            point(x, brush.response(x))
        })
        .collect();
    let blue = egui::Color32::from_rgb(78, 132, 204);
    ui.painter()
        .add(egui::Shape::line(points, egui::Stroke::new(2.0_f32, blue)));
    if brush.custom_pressure {
        for index in 1..4 {
            ui.painter().circle_filled(
                point(index as f32 / 4.0, brush.pressure_curve[index]),
                6.0,
                blue,
            );
        }
        if ui.button("Reset curve to linear").clicked() {
            brush.pressure_curve = [0.0, 0.25, 0.5, 0.75, 1.0];
        }
        ui.label("Drag the three handles vertically. Left → right: pen pressure; bottom → top: brush response.");
    }
    if let Some(pressure) = pressure {
        ui.painter().circle_filled(
            point(pressure.clamp(0.0, 1.0), brush.response(pressure)),
            4.0,
            egui::Color32::WHITE,
        );
    }
}
