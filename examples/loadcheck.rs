//! `cargo run --example loadcheck -- file.pdf`: time how long the page list takes to load.
fn main() {
    let Some(p) = std::env::args().nth(1) else {
        eprintln!("usage: loadcheck FILE.pdf");
        std::process::exit(2);
    };
    let t = std::time::Instant::now();
    match mizu::doc::load(std::path::Path::new(&p), None) {
        Ok(i) => println!("ok {} pages in {:?}", i.pages.len(), t.elapsed()),
        Err(e) => println!("err {e} in {:?}", t.elapsed()),
    }
}
