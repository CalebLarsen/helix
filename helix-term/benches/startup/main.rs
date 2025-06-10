mod helpers;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use helpers::test_key_sequence;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

static RT: OnceLock<Runtime> = OnceLock::new();

fn startup() {
    let rt = RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(1)
            .build()
            .unwrap()
    });
    rt.block_on(async {
        let mut app = helpers::AppBuilder::new().build().unwrap();
        let res = test_key_sequence(&mut app, Some("i"), None, false).await;
        match res {
            Ok(_) => (),
            Err(e) => panic!("{}", e),
        }
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("Start Up", |b| {
        b.iter(|| {
            black_box(());
            startup();
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
