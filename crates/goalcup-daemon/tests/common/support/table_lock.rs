//! Process-table tests must not run concurrently with each other.
//!
//! Every consumer of this module spawns a real process tree and reads it back through
//! `/bin/ps`. Run several at once and `ps` itself starts failing to spawn; in
//! `goalcup-daemon` that surfaces as `guard_registered`/`snapshot` returning `Err`,
//! `sweep_anchor` folding it to `Anchor::Lost` and the escalation failing closed — which is
//! the fail-closed rule working exactly as specified, and is also indistinguishable, from inside a
//! test, from the defect the test is looking for. In `goalcup-cli` the same contention
//! surfaces as `ProcessAuthorityUnavailable`.
//!
//! A `static` here is per-process, not global. Under plain `cargo test` that is enough: cargo
//! runs test targets one at a time, so the only contention is a binary against its own worker
//! pool, and this lock serialises that. Under cargo-nextest (`sh scripts/gate.sh`) every test
//! is its own process and this lock serialises nothing. There, safety comes from the `serial`
//! test group in `.config/nextest.toml`: **any new module that takes this lock must be added
//! to that group's filters**, or its tests will run concurrently. `acquire` enforces this:
//! under nextest it panics unless `NEXTEST_TEST_GROUP` is `serial`, so a module that was
//! missed fails loudly instead of flaking on `ps`.
//!
//! Never acquire this from a thread the test spawned. The depth counter is
//! thread-local, so a guard held on the test's main thread plus an acquisition on a worker
//! thread deadlocks: the worker's count is 0, so it blocks on the mutex the main thread
//! holds. Guard test bodies and spawn helpers that run on the test's own thread only.
//!
//! Declared by `tests/common/mod.rs` as `common::table_lock`, beside `support` rather than
//! inside it.

use std::cell::RefCell;
use std::sync::{Mutex, MutexGuard};

static PROCESS_TABLE: Mutex<()> = Mutex::new(());

thread_local! {
    static HELD: RefCell<(usize, Option<MutexGuard<'static, ()>>)> =
        const { RefCell::new((0, None)) };
}

/// Held for as long as a spawned tree exists. Reentrant per thread, so a test that holds
/// two trees at once takes it twice and releases on the outer drop.
///
/// The field is private and `PhantomData<*const ()>` makes it `!Send`. Both matter: a
/// publicly constructible guard could be dropped without ever being acquired, underflowing
/// the depth counter, and a `Send` one would let an owning struct move to another thread and
/// try to release a `MutexGuard` still held in the acquiring thread's TLS.
///
/// Poisoning is ignored. The only state under the mutex is `()`, and a test that panicked
/// under the guard had its tree killed by its own owner's drop before this was released.
pub struct TableLock(std::marker::PhantomData<*const ()>);

/// Why a test in nextest group `group` may not take the lock, or `None` when it may. nextest sets
/// `NEXTEST_TEST_GROUP` to the test's group, or `@global` for a test in no group; plain
/// `cargo test` leaves it unset, and a single binary's worker pool is serialised by the static.
pub fn group_refusal(group: Option<&str>) -> Option<String> {
    match group {
        None | Some("serial") => None,
        Some(other) => Some(format!(
            "TableLock::acquire() ran in nextest test group `{other}`, not `serial`: add this \
             module to the `serial` filters in .config/nextest.toml, or the lock holds nothing \
             and process-table tests run concurrently"
        )),
    }
}

impl TableLock {
    pub fn acquire() -> TableLock {
        if let Some(refusal) = group_refusal(std::env::var("NEXTEST_TEST_GROUP").ok().as_deref()) {
            panic!("{refusal}");
        }
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            if held.0 == 0 {
                held.1 = Some(PROCESS_TABLE.lock().unwrap_or_else(|e| e.into_inner()));
            }
            held.0 += 1;
        });
        TableLock(std::marker::PhantomData)
    }
}

impl Drop for TableLock {
    fn drop(&mut self) {
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            held.0 -= 1;
            if held.0 == 0 {
                held.1 = None;
            }
        });
    }
}
