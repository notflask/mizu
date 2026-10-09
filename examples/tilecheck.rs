use mizu::doc::worker::{Pool, Rendered, TileKey};
use std::sync::Arc;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let scale: f32 = std::env::args().nth(2).unwrap().parse().unwrap();
    let pool = Pool::spawn(path.into(), None, Arc::new(|| {}), 1);
    let key = TileKey {
        page: 0,
        scale: scale.to_bits(),
        tx: 1,
        ty: 0,
    };
    pool.set_wanted(vec![key], vec![]);
    if let Ok(Rendered::Tile(t)) = pool.rx.recv() {
        println!("valid {}x{}", t.w, t.h);
        for x in (t.w as usize - 4)..(t.w as usize + 3) {
            let i = (300 * 512 + x) * 4;
            println!("x={x} {:?}", &t.data[i..i + 4]);
        }
    }
}
