#![no_main]

use core::str;
use tokio::runtime::Runtime;

use helpers::test_key_sequence;
#[macro_use]
extern crate libfuzzer_sys;

mod helpers;

use std::sync::OnceLock;

static RT: OnceLock<Runtime> = OnceLock::new();
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = str::from_utf8(data) {
        if s.is_empty() {
            return;
        }
        let mut parsed = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '\x3c' => parsed.push_str("<lt>"),
                '\x3e' => parsed.push_str("<gt>"),
                '\x0a' => parsed.push_str("<ret>"),
                '\x1b' => parsed.push_str("<esc>"),
                _ => parsed.push(c),
            };
        }
        let rt = RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(1)
                .build()
                .unwrap()
        });
        rt.block_on(async {
            let mut app = helpers::AppBuilder::new().build().unwrap();
            let res = test_key_sequence(&mut app, Some(&parsed), None, false).await;
            match res {
                Ok(_) => (),
                Err(e) => panic!("{}", e),
            }
        });
    }
});
