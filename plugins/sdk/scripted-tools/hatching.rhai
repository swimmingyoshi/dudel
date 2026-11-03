// Hatching: short parallel strokes laid down along the path. Press harder
// for longer strokes. Each stroke ends with `lift`, so they never join.

fn begin(x, y, pressure) {
    this.spacing = max(this.brush_size * 1.2, 3.0);
    this.length = max(this.brush_size * 5.0, 12.0);
    // Distance travelled since the last hatch.
    this.travel = this.spacing;
    this.last_x = x;
    this.last_y = y;
    []
}

fn point(x, y, pressure) {
    let mx = x - this.last_x;
    let my = y - this.last_y;
    let moved = (mx * mx + my * my).sqrt();
    let points = [];
    // A fast pen covers several hatches between two samples: place each one
    // where it belongs along the segment.
    let walked = 0.0;
    while walked + (this.spacing - this.travel) <= moved {
        walked += this.spacing - this.travel;
        this.travel = 0.0;
        let t = if moved > 0.0 { walked / moved } else { 0.0 };
        let cx = this.last_x + mx * t;
        let cy = this.last_y + my * t;
        // At 45 degrees, centred on the path; longer with more pressure.
        let half = this.length * (0.4 + 0.6 * pressure) / 2.0;
        let d = half * 0.7071;
        points.push(#{ x: cx - d, y: cy + d, pressure: pressure });
        points.push(#{ x: cx + d, y: cy - d, pressure: pressure * 0.6, lift: true });
    }
    this.travel += moved - walked;
    this.last_x = x;
    this.last_y = y;
    points
}
