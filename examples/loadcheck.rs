fn main() {
    let p = std::env::args().nth(1).unwrap();
    let t = std::time::Instant::now();
    match mizu::doc::load(std::path::Path::new(&p), None) {
        Ok(i) => println!(
            "ok {} pages in {:?}: {:?}",
            i.pages.len(),
            t.elapsed(),
            i.pages
        ),
        Err(e) => println!("err {e} in {:?}", t.elapsed()),
    }
}
