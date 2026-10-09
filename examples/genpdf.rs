//! `cargo run --example genpdf -- out.pdf [pages]`: sample PDF for manual testing.
use mizu::testutil::{make_pdf, Extras, PageSpec};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("usage: genpdf OUT [PAGES]");
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(12);
    let pages: Vec<PageSpec> = (0..n)
        .map(|i| {
            PageSpec::new(
                595.0,
                842.0,
                &format!("Page {} of {} hello world", i + 1, n),
            )
        })
        .collect();
    std::fs::write(
        &out,
        make_pdf(
            &pages,
            &Extras {
                outline: true,
                links: true,
            },
        ),
    )
    .unwrap();
}
