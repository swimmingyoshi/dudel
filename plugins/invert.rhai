// Dudel filter API v1. Channels are integers from 0 to 255.
fn filter(r, g, b, a) {
    [255 - r, 255 - g, 255 - b, a]
}
