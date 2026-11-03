// Coil: a line that loops around the path as you draw, like a spring or a
// telephone cord. The loops are spaced by distance, not by time, so slow and
// fast strokes look alike.

fn begin(x, y, pressure) {
    this.radius = max(this.brush_size * 2.5, 8.0);
    // How far along the path one loop takes.
    this.pitch = this.radius * 1.6;
    this.angle = 0.0;
    this.last_x = x;
    this.last_y = y;
    [[x + this.radius, y, pressure]]
}

fn point(x, y, pressure) {
    let mx = x - this.last_x;
    let my = y - this.last_y;
    let moved = (mx * mx + my * my).sqrt();
    let points = [];
    // Step along the segment, at most 2 px and 12 degrees of turn at a
    // time, so long jumps still draw round loops.
    let turn = moved / this.pitch * 2.0 * PI();
    let steps = max(max(moved / 2.0, turn / 0.21).ceiling().to_int(), 1);
    for i in 1..=steps {
        let t = i.to_float() / steps.to_float();
        this.angle += turn / steps.to_float();
        let r = this.radius * (0.5 + 0.5 * pressure);
        points.push([
            this.last_x + mx * t + r * this.angle.cos(),
            this.last_y + my * t + r * this.angle.sin(),
            pressure,
        ]);
    }
    this.last_x = x;
    this.last_y = y;
    points
}
