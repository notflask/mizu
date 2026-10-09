//! `cargo run --example genepub -- out.epub`: sample EPUB for manual testing.
fn main() {
    let out = std::env::args().nth(1).expect("usage: genepub OUT");
    let text = "It was a bright cold day in April, and the clocks were striking thirteen. \
                The river ran on under the bridge, green and quiet, carrying the light with it.";
    let chapters = [
        ("One", text),
        ("Two", text),
        ("Three", text),
        ("Four", text),
    ];
    std::fs::write(&out, mizu::testutil::make_epub(&chapters)).expect("write");
}
