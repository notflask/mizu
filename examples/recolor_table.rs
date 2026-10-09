//! Prints the CPU reference output for a few colours (used to cross-check the shader).
use mizu::render::recolor::{recolor_srgb8, DarkTheme};
fn main() {
    let t = DarkTheme::from_srgb8([255, 255, 255], [0, 0, 0]);
    for (r, g, b) in [
        (1.0f32, 0.0f32, 0.0f32),
        (0.0, 0.6, 0.0),
        (0.0, 0.0, 1.0),
        (0.5, 0.5, 0.5),
        (0.1, 0.1, 0.1),
        (0.9, 0.9, 0.9),
        (1.0, 0.5, 0.0),
        (0.6, 0.2, 0.8),
    ] {
        let c = [
            (r * 255.0).round() as u8,
            (g * 255.0).round() as u8,
            (b * 255.0).round() as u8,
        ];
        println!("{:?} -> {:?}", c, recolor_srgb8(c, &t));
    }
}
