fn filter(r, g, b, a) {
    let gray = (r * 30 + g * 59 + b * 11) / 100;
    [gray, gray, gray, a]
}
