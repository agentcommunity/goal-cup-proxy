//! What the CLI tests share: `status.json` (seam S4), `upload.json` (S9) and `server.json`
//! fixtures built through their JSON form, so a key PRD-02 or PRD-06 adds changes one place, and
//! the part folder to write them into. Options left out read as null.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use goalcup::cli::status::Ctx;
use goalcup::pipeline::status::{DurableStatus, Losing, Mode, Off, STATUS_V, Status, Written};
use goalcup::upload::state::UploadJson;
use serde::Serialize;
use serde_json::{Value, json};

/// 2026-11-07T10:00:00Z, the `updated` of [`status`].
pub const T0: u64 = 1_794_045_600;
pub const RUN: &str = "r0123456789abcdef";
pub const ENTRY: &str = "e0123456789abcdef";

/// A `status.json` in `mode`, written at [`T0`], with nothing counted.
pub fn status(mode: Mode) -> Status {
    serde_json::from_value(json!({
        "v": STATUS_V, "updated": "2026-11-07T10:00:00Z", "pid": 0, "start_id": 0, "boot": 0,
        "port": 41_000, "mode": mode, "written": Written::default(), "losing": Losing::default(),
        "off": Off::default(), "notes": [], "held_bytes": 0, "held_peak": 0, "guard": 0,
        "writer_idle_ms": 0, "durable": DurableStatus::default(),
    }))
    .expect("a status.json")
}

/// [`status`] in `mode` with `p` laid over it, as [`patch`] does.
pub fn with(mode: Mode, p: Value) -> Status {
    patch(&status(mode), p)
}

/// `st` with `p` laid over its JSON form object by object: `{"written": {"gaps": 1}}` sets one
/// count and keeps the rest; a null clears an option.
pub fn patch(st: &Status, p: Value) -> Status {
    let mut v = serde_json::to_value(st).unwrap();
    merge(&mut v, p);
    serde_json::from_value(v).expect("a status.json")
}

/// The `upload.json` of part [`ENTRY`] with `p` laid over it, as [`patch`] does.
pub fn upload(p: Value) -> UploadJson {
    let mut v = serde_json::to_value(UploadJson::new(RUN.into(), ENTRY.into())).unwrap();
    merge(&mut v, p);
    serde_json::from_value(v).expect("an upload.json")
}

fn merge(to: &mut Value, from: Value) {
    match (to, from) {
        (Value::Object(to), Value::Object(from)) => {
            for (k, v) in from {
                merge(to.entry(k).or_insert(Value::Null), v);
            }
        }
        (to, from) => *to = from,
    }
}

/// The `server.json` of a practice run [`RUN`] with the one part [`ENTRY`].
pub fn server_json() -> Value {
    json!({
        "v": 1, "server": null, "run": RUN, "mode": "practice", "deadline": null, "repo": null,
        "connector": {"id": "agent", "name": "Agent", "source": "builtin"}, "hosts_file": null,
        "entries": [{"id": ENTRY, "harness_version": null, "resumed": false}],
    })
}

/// The folder of part [`ENTRY`] under the goalcup home `home`.
pub fn part(home: &Path) -> PathBuf {
    home.join("runs").join(RUN).join(ENTRY)
}

/// Writes `value` as JSON to `dir/name`, making `dir` first.
pub fn put(dir: &Path, name: &str, value: &impl Serialize) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), serde_json::to_vec(value).unwrap()).unwrap();
}

/// Liveness and now (`T0 + age`); the run is open for a day.
pub fn live(recorder: bool, watchdog: bool, harness: bool, age: u64) -> Ctx<'static> {
    Ctx {
        name: "Agent",
        run: RUN,
        recorder,
        watchdog,
        harness,
        deadline: Some(T0 + 86_400),
        now: T0 + age,
    }
}
