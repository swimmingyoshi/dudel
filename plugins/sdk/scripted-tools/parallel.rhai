// Parallel lines: three lines that follow the pen side by side, like rails
// or speed lines. Each line is its own track, so the brush draws each one
// with its own spacing, smoothing and pressure.

fn begin(x, y, pressure) {
    // `this` is a map kept for the whole stroke; the host fills in
    // canvas_width, canvas_height, brush_size and zoom.
    this.gap = max(this.brush_size * 1.5, 4.0);
    this.last_x = x;
    this.last_y = y;
    this.dx = 0.0;
    this.dy = 0.0;
    []
}

fn point(x, y, pressure) {
    let mx = x - this.last_x;
    let my = y - this.last_y;
    let moved = (mx * mx + my * my).sqrt();
    // Wait until the pen has moved enough to know which way it goes.
    if moved < 1.0 {
        return [];
    }
    // Ease the direction so the side lines do not wobble on small jitters.
    this.dx = this.dx * 0.6 + mx / moved * 0.4;
    this.dy = this.dy * 0.6 + my / moved * 0.4;
    let length = (this.dx * this.dx + this.dy * this.dy).sqrt();
    this.last_x = x;
    this.last_y = y;
    // Perpendicular to the direction of travel.
    let nx = -this.dy / length;
    let ny = this.dx / length;
    let points = [];
    for side in [-1.0, 0.0, 1.0] {
        let track = side.to_int() + 1;
        points.push([x + nx * side * this.gap, y + ny * side * this.gap, pressure, track]);
    }
    points
}
