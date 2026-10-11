//! small shared helpers, threads, paths and a stable hash.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

/// run f over items on jobs threads and keep the order.
pub fn par_map<T: Send, R: Send>(items: Vec<T>, jobs: usize, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let n = items.len();
    let work: Mutex<Vec<(usize, T)>> = Mutex::new(items.into_iter().enumerate().rev().collect());
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, n.max(1)) {
            scope.spawn(|| {
                loop {
                    let next = work.lock().unwrap().pop();
                    let Some((i, item)) = next else { break };
                    let r = f(item);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every item was processed"))
        .collect()
}

/// tools/, this crate lives in tools/gen.
pub fn tools_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tools/gen has a parent")
        .to_path_buf()
}

/// FNV-1a, same on every machine.
pub fn fnv(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn par_map_keeps_the_order_whatever_the_thread_count() {
        let input: Vec<u32> = (0..50).collect();
        for jobs in [1, 4, 16] {
            assert_eq!(
                par_map(input.clone(), jobs, |x| x * 2),
                (0..50).map(|x| x * 2).collect::<Vec<_>>()
            );
        }
        assert!(par_map(Vec::<u32>::new(), 4, |x| x).is_empty());
    }
}
