//! The fake Worker (PRD-14 section 5): `docs/UPLOAD-API.md` (seam S5) answered from its own
//! position, with injected answers; it keeps every chunk body and heartbeat and has no continuity
//! rules (the full tier runs the real Worker). HTTP/1.1, keep-alive, any Bearer; an entry is made
//! on first use; ids, the credential and times are fixed test data (`seconds_left` too).
//!
//! A heartbeat is kept as the Worker keeps it: a `counts` outside the S5 bound (a flat object of
//! whole numbers, strings of at most 64 bytes, booleans or `null`, at most 2 KiB serialized) is
//! dropped and kept as `counts_dropped` (its serialized byte length), answered `200`, never `400`.
//!
//! An outage: dropping the fake shuts down every connection it accepted (a pooled keep-alive
//! connection sees the close) and frees the port; `ports::refusing()` gives an address that
//! refuses from the start. A request with `Connection: close` gets its answer, then the close.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex, MutexGuard, atomic::AtomicBool, atomic::Ordering::SeqCst};
use std::thread::{JoinHandle, sleep, spawn};
use std::{collections::HashMap, time::Duration};

pub const RUN_ID: &str = "r00000000000000a1";
pub const CREDENTIAL: &str = "gcr_000000000000000000000000000000000000000000000000000000000000fa1e";
pub const SECONDS_LEFT: u64 = 86_400;
const START: &str = "2026-11-06T10:00:00Z";
const DEADLINE: &str = "2026-11-07T10:00:00Z";

/// A route of the upload API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    /// `POST /v1/device`
    Device,
    /// `POST /v1/device/token`
    DeviceToken,
    /// `POST /v1/runs`
    Open,
    /// `GET /v1/runs/{run_id}`
    Position,
    /// `PUT /v1/runs/{run_id}/chunks/{offset}`
    Chunk,
    /// `POST /v1/runs/{run_id}/heartbeat`
    Heartbeat,
    /// `POST /v1/runs/{run_id}/continue`
    Continue,
}

/// One injected answer, used once, for the next call of its route.
#[derive(Clone, Debug, Default)]
pub struct Inject {
    /// Answer with this status and body instead, doing no work: the test names the documented
    /// code it means, for example `(503, json!({"error": "awaiting_final"}))`.
    pub answer: Option<(u16, Value)>,
    /// Do the work, then close the connection unanswered.
    pub drop_after_commit: bool,
    /// A `Retry-After` header, in seconds.
    pub retry_after: Option<u64>,
    /// Wait this long first.
    pub delay: Option<Duration>,
}

#[derive(Clone, Debug)]
pub struct Chunk {
    pub entry_id: String,
    pub offset: u64,
    pub body: Vec<u8>,
}

#[derive(Default)]
struct State {
    script: Vec<(Route, Inject)>,
    chunks: Vec<Chunk>,
    heartbeats: Vec<Value>,
    /// Each entry's position answer, without `status`, `sealed` and `stop`.
    entries: Vec<Value>,
    stop: bool,
    /// A handle on every accepted connection, shut down when the fake is dropped.
    conns: Vec<TcpStream>,
}

/// Locks `m`, passing a poisoned lock on so a test sees the first panic, not a `PoisonError`.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The entry named `id`, made on first use as the run's next segment.
fn entry<'a>(entries: &'a mut Vec<Value>, id: &str) -> &'a mut Value {
    if let Some(i) = entries.iter().position(|e| e["entry_id"] == id) {
        return &mut entries[i];
    }
    let e = json!({"run_id": RUN_ID, "entry_id": id, "segment": entries.len(),
        "official_start": START, "deadline_at": DEADLINE, "ends_at": DEADLINE,
        "offset": 0, "next_seq": 0, "head": "0".repeat(64), "final": null});
    entries.push(e);
    entries.last_mut().expect("just pushed")
}

/// Sealed once its committed end (`final`) is the last LINK it holds.
fn sealed(e: &Value) -> bool {
    let end = e["final"]["seq"].as_u64().map(|seq| seq + 1) == e["next_seq"].as_u64();
    end && e["final"]["head"] == e["head"]
}

/// A running fake Worker on a held loopback port. Drop stops its accept loop and shuts down every
/// connection it accepted.
pub struct FakeWorker {
    url: String,
    state: Arc<Mutex<State>>,
    stop: Option<(SocketAddr, Arc<AtomicBool>, JoinHandle<()>)>,
}

impl FakeWorker {
    /// Starts the fake: an accept thread on a held port, and one thread per connection.
    pub fn start() -> FakeWorker {
        let (listener, addr) = super::ports::held();
        let (state, done): (Arc<Mutex<State>>, Arc<AtomicBool>) = Default::default();
        let (st, dn) = (state.clone(), done.clone());
        let accept = spawn(move || {
            for stream in listener
                .incoming()
                .take_while(|_| !dn.load(SeqCst))
                .flatten()
            {
                let Ok(handle) = stream.try_clone() else {
                    continue;
                };
                lock(&st).conns.push(handle);
                let st = st.clone();
                spawn(move || {
                    serve(&stream, &st);
                    // The handle kept in `conns` holds the socket open: close it here.
                    let _ = stream.shutdown(Shutdown::Both);
                });
            }
        });
        let (url, stop) = (format!("http://{addr}"), Some((addr, done, accept)));
        FakeWorker { url, state, stop }
    }
    /// The base URL, `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        self.url.clone()
    }
    /// Queues `inject` for the next call of `route` (in the order queued).
    pub fn inject(&self, route: Route, inject: Inject) {
        lock(&self.state).script.push((route, inject));
    }
    /// Every stored chunk, per entry in the order stored.
    pub fn chunks(&self) -> Vec<Chunk> {
        lock(&self.state).chunks.clone()
    }
    /// The stored bodies of `entry_id`, joined in order: its compressed record stream.
    pub fn stream(&self, entry_id: &str) -> Vec<u8> {
        let chunks = self.chunks().into_iter().filter(|c| c.entry_id == entry_id);
        chunks.flat_map(|c| c.body).collect()
    }
    pub fn heartbeats(&self) -> Vec<Value> {
        lock(&self.state).heartbeats.clone()
    }
    /// The run is over from now (`stop: true`, `seconds_left` 0, reason `deadline`), or not.
    pub fn set_stop(&self, stop: bool) {
        lock(&self.state).stop = stop;
    }
}

impl Drop for FakeWorker {
    fn drop(&mut self) {
        let (addr, done, accept) = self.stop.take().expect("set by start");
        done.store(true, SeqCst);
        TcpStream::connect(addr).map(|_| accept.join()).ok();
        for conn in lock(&self.state).conns.drain(..) {
            let _ = conn.shutdown(Shutdown::Both);
        }
    }
}

/// One request, header names in lower case.
struct Request {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

/// Serves one connection, request after request, until the peer closes it, it asks to close, an
/// answer is dropped, or the fake is dropped.
fn serve(stream: &TcpStream, state: &Mutex<State>) -> Option<()> {
    let mut r = BufReader::new(stream);
    loop {
        let mut lines = r.by_ref().lines().map_while(Result::ok);
        let start = lines.next()?;
        let mut start = start.split(' ').map(str::to_string);
        let mut headers = HashMap::new();
        for line in lines.take_while(|l| !l.is_empty()) {
            let (k, v) = line.split_once(':')?;
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
        let len = headers.get("content-length").map_or("0", String::as_str);
        let mut req = Request {
            method: start.next()?,
            target: start.next()?,
            body: vec![0; len.parse().ok()?],
            headers,
        };
        r.read_exact(&mut req.body).ok()?;
        let (status, retry, body) = answer(&req, state)?;
        let mut head = format!("HTTP/1.1 {status} Fake\r\nContent-Length: {}", body.len());
        head.extend(retry.map(|s| format!("\r\nRetry-After: {s}")));
        let all = format!("{head}\r\nContent-Type: application/json\r\n\r\n{body}");
        (&*stream).write_all(all.as_bytes()).ok()?;
        if req
            .headers
            .get("connection")
            .is_some_and(|v| v.eq_ignore_ascii_case("close"))
        {
            return None;
        }
    }
}

/// The fake's own errors: `404 not_found` for an unknown route or entry, else `400 malformed`.
fn own_error(status: u16) -> Value {
    let code = if status == 404 {
        "not_found"
    } else {
        "malformed"
    };
    json!({"error": code, "message": format!("The fake Worker answered {status}.")})
}

/// Whether `c` is within the S5 `counts` bound.
fn counts_ok(c: &Value) -> bool {
    let flat = c.as_object().is_some_and(|m| {
        m.values().all(|v| match v {
            Value::Number(n) => n.is_u64() || n.is_i64(),
            Value::String(s) => s.len() <= 64,
            Value::Bool(_) | Value::Null => true,
            _ => false,
        })
    });
    flat && c.to_string().len() <= 2048
}

/// The answer to `req`: status, `Retry-After` and body; `None` drops the connection unanswered.
fn answer(req: &Request, state: &Mutex<State>) -> Option<(u16, Option<u64>, String)> {
    let (path, q) = req.target.split_once('?').unwrap_or((&req.target, ""));
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    let route = match (req.method.as_str(), segs.as_slice()) {
        ("GET", ["v1", "health"]) => return Some((200, None, json!({"status": "ok"}).to_string())),
        ("POST", ["v1", "device"]) => Route::Device,
        ("POST", ["v1", "device", "token"]) => Route::DeviceToken,
        ("POST", ["v1", "runs"]) => Route::Open,
        ("GET", ["v1", "runs", _]) => Route::Position,
        ("PUT", ["v1", "runs", _, "chunks", _]) => Route::Chunk,
        ("POST", ["v1", "runs", _, "heartbeat"]) => Route::Heartbeat,
        ("POST", ["v1", "runs", _, "continue"]) => Route::Continue,
        _ => return Some((404, None, own_error(404).to_string())),
    };
    let mut s = lock(state);
    let at = s.script.iter().position(|(r, _)| *r == route);
    let inject = at.map(|i| s.script.remove(i).1).unwrap_or_default();
    drop(s);
    sleep(inject.delay.unwrap_or_default());
    if let Some((status, body)) = inject.answer {
        return Some((status, inject.retry_after, body.to_string()));
    }
    let created = matches!(route, Route::Open | Route::Continue);
    let ok = if created { 201 } else { 200 };
    let body = work(route, req, &segs, q, &mut lock(state));
    let (status, body) = body.map_or_else(|e| (e, own_error(e)), |b| (ok, b));
    (!inject.drop_after_commit).then_some((status, inject.retry_after, body.to_string()))
}

/// The normal answer of `route`, or the status of a missing or malformed field (`400`).
fn work(r: Route, req: &Request, segs: &[&str], q: &str, s: &mut State) -> Result<Value, u16> {
    let (stop, left) = (s.stop, if s.stop { 0 } else { SECONDS_LEFT });
    let h = |name: &str| req.headers.get(name).map(String::as_str).ok_or(400u16);
    let num = |name| h(name)?.parse::<u64>().map_err(|_| 400u16);
    Ok(match r {
        Route::Device => json!({"device_code": "de".repeat(32), "user_code": "BCDF-GHJK",
            "verification_uri": "http://127.0.0.1/approve", "interval": 1, "expires_in": 900,
            "verification_uri_complete": "http://127.0.0.1/approve?code=BCDF-GHJK"}),
        Route::DeviceToken => json!({"status": "approved", "purpose": "run",
            "credential": CREDENTIAL, "expires_at": DEADLINE}),
        Route::Open | Route::Continue => {
            let n = s.entries.len() + 1;
            let id = format!("e{:016x}", if r == Route::Open { 1 } else { n });
            let e = entry(&mut s.entries, &id);
            json!({"run_id": RUN_ID, "entry_id": e["entry_id"], "segment": e["segment"],
                "official_start": START, "deadline_at": DEADLINE, "seconds_left": left})
        }
        Route::Position => {
            let named = q.split('&').find_map(|q| q.strip_prefix("entry_id="));
            let last = s.entries.last().map(|e| e["entry_id"].clone());
            let id = named.map(Value::from).or(last).ok_or(404u16)?;
            let mut e = entry(&mut s.entries, id.as_str().unwrap_or_default()).clone();
            let is_sealed = sealed(&e);
            e["status"] = if stop && is_sealed { "sealed" } else { "open" }.into();
            e["sealed"] = is_sealed.into();
            e["stop"] = stop.into();
            e
        }
        Route::Chunk => {
            let offset: u64 = segs[4].parse().map_err(|_| 400u16)?;
            let (len, last) = (num("goalcup-length")?, num("goalcup-last-seq")?);
            let (id, head) = (h("goalcup-entry-id")?, h("goalcup-head")?);
            s.chunks.push(Chunk {
                entry_id: id.into(),
                offset,
                body: req.body.clone(),
            });
            let e = entry(&mut s.entries, id);
            e["offset"] = (e["offset"].as_u64().unwrap_or(0) + len).into();
            e["next_seq"] = (last + 1).into();
            e["head"] = head.into();
            json!({"offset": e["offset"], "next_seq": e["next_seq"], "head": e["head"],
                "received_at": "2026-11-06T10:00:01Z", "sealed": sealed(e)})
        }
        Route::Heartbeat => {
            let mut beat: Value = serde_json::from_slice(&req.body).map_err(|_| 400u16)?;
            let id = beat["entry_id"].as_str().ok_or(400u16)?.to_string();
            let bad = beat.get("counts").filter(|c| !c.is_null() && !counts_ok(c));
            if let (Some(n), Some(o)) = (bad.map(|c| c.to_string().len()), beat.as_object_mut()) {
                o.remove("counts");
                o.insert("counts_dropped".into(), n.into());
            }
            let e = entry(&mut s.entries, &id);
            if beat["final"] == true {
                e["final"] = json!({"seq": beat["seq"], "head": beat["head"]});
            }
            s.heartbeats.push(beat);
            json!({"stop": stop, "deadline_at": DEADLINE, "seconds_left": left,
                "reason": stop.then_some("deadline"), "sealed": sealed(e)})
        }
    })
}
