//! The fake Worker (PRD-14 section 5): `docs/UPLOAD-API.md` (seam S5) answered from its own
//! position, with injected answers; it keeps every chunk body and heartbeat and has no continuity
//! rules (the full tier runs the real Worker). HTTP/1.1, keep-alive, any Bearer; an entry is made
//! on first use; ids, the credential and times are fixed test data (`seconds_left` too).

use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex, atomic::AtomicBool, atomic::Ordering::SeqCst};
use std::thread::{JoinHandle, sleep, spawn};
use std::{collections::HashMap, net::SocketAddr, net::TcpStream, time::Duration};

pub const RUN_ID: &str = "r00000000000000a1";
pub const CREDENTIAL: &str = "gcr_000000000000000000000000000000000000000000000000000000000000fa1e";
pub const SECONDS_LEFT: u64 = 86_400;
const START: &str = "2026-11-06T10:00:00Z";
const DEADLINE: &str = "2026-11-07T10:00:00Z";
const CODES: &str = "400:malformed 401:unauthorized 403:forbidden 404:not_found 409:conflict \
    410:closed 413:too_large 422:digest_mismatch 429:rate_limited 503:maintenance";

/// A route of the upload API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Device,      // POST /v1/device
    DeviceToken, // POST /v1/device/token
    Open,        // POST /v1/runs
    Position,    // GET /v1/runs/{run_id}
    Chunk,       // PUT /v1/runs/{run_id}/chunks/{offset}
    Heartbeat,   // POST /v1/runs/{run_id}/heartbeat
    Continue,    // POST /v1/runs/{run_id}/continue
}

/// One injected answer, used once, for the next call of its route.
#[derive(Clone, Debug, Default)]
pub struct Inject {
    pub status: Option<u16>, // answer it with an error body instead, doing no work
    pub drop_after_commit: bool, // do the work, then close the connection unanswered
    pub retry_after: Option<u64>, // a `Retry-After` header, in seconds
    pub delay: Option<Duration>, // wait this long first
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
    entries: Vec<Value>, // each entry's position answer, without `status`, `sealed` and `stop`
    stop: bool,
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

/// A running fake Worker on a held loopback port. Drop wakes its accept loop and waits for it.
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
            let live = listener.incoming().take_while(|_| !dn.load(SeqCst));
            for (s, st) in live.flatten().zip(std::iter::repeat(st)) {
                spawn(move || serve(s, &st));
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
        self.state.lock().unwrap().script.push((route, inject));
    }
    /// Every stored chunk, per entry in the order stored.
    pub fn chunks(&self) -> Vec<Chunk> {
        self.state.lock().unwrap().chunks.clone()
    }
    /// The stored bodies of `entry_id`, joined in order: its compressed record stream.
    pub fn stream(&self, entry_id: &str) -> Vec<u8> {
        let chunks = self.chunks().into_iter().filter(|c| c.entry_id == entry_id);
        chunks.flat_map(|c| c.body).collect()
    }
    pub fn heartbeats(&self) -> Vec<Value> {
        self.state.lock().unwrap().heartbeats.clone()
    }
    /// The run is over from now (`stop: true`, `seconds_left` 0, reason `deadline`), or not.
    pub fn set_stop(&self, stop: bool) {
        self.state.lock().unwrap().stop = stop;
    }
}

impl Drop for FakeWorker {
    fn drop(&mut self) {
        let (addr, done, accept) = self.stop.take().expect("set by start");
        done.store(true, SeqCst);
        TcpStream::connect(addr).map(|_| accept.join()).ok();
    }
}

type Request = (String, String, HashMap<String, String>, Vec<u8>); // headers in lower case
/// Serves one connection, request after request, until the peer closes it or an answer is dropped.
fn serve(stream: TcpStream, state: &Mutex<State>) -> Option<()> {
    let mut r = BufReader::new(stream.try_clone().ok()?);
    loop {
        let mut lines = r.by_ref().lines().map_while(Result::ok);
        let start = lines.next()?;
        let (mut start, mut headers) = (start.split(' ').map(str::to_string), HashMap::new());
        for line in lines.take_while(|l| !l.is_empty()) {
            let (k, v) = line.split_once(':')?;
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
        let len: &str = headers.get("content-length").map_or("0", String::as_str);
        let body = vec![0; len.parse().ok()?];
        let mut req = (start.next()?, start.next()?, headers, body);
        r.read_exact(&mut req.3).ok()?;
        let (status, retry, body) = answer(&req, state)?;
        let mut head = format!("HTTP/1.1 {status} Fake\r\nContent-Length: {}", body.len());
        head.extend(retry.map(|s| format!("\r\nRetry-After: {s}")));
        let all = format!("{head}\r\nContent-Type: application/json\r\n\r\n{body}");
        (&stream).write_all(all.as_bytes()).ok()?;
    }
}

fn error(status: u16) -> Value {
    let (mut codes, s) = (CODES.split(' '), format!("{status}:"));
    let code = codes.find_map(|c| c.strip_prefix(&s)).unwrap_or("internal");
    json!({"error": code, "message": format!("The fake Worker answered {status}.")})
}

/// The answer to `req`: status, `Retry-After` and body; `None` drops the connection unanswered.
fn answer(req: &Request, state: &Mutex<State>) -> Option<(u16, Option<u64>, String)> {
    let (path, q) = req.1.split_once('?').unwrap_or((&req.1, ""));
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    let route = match (req.0.as_str(), segs.as_slice()) {
        ("GET", ["v1", "health"]) => return Some((200, None, json!({"status": "ok"}).to_string())),
        ("POST", ["v1", "device"]) => Route::Device,
        ("POST", ["v1", "device", "token"]) => Route::DeviceToken,
        ("POST", ["v1", "runs"]) => Route::Open,
        ("GET", ["v1", "runs", _]) => Route::Position,
        ("PUT", ["v1", "runs", _, "chunks", _]) => Route::Chunk,
        ("POST", ["v1", "runs", _, "heartbeat"]) => Route::Heartbeat,
        ("POST", ["v1", "runs", _, "continue"]) => Route::Continue,
        _ => return Some((404, None, error(404).to_string())),
    };
    let mut s = state.lock().unwrap();
    let at = s.script.iter().position(|(r, _)| *r == route);
    let inject = at.map(|i| s.script.remove(i).1).unwrap_or_default();
    drop(s);
    sleep(inject.delay.unwrap_or_default());
    if let Some(status) = inject.status {
        return Some((status, inject.retry_after, error(status).to_string()));
    }
    let ok = 200 + u16::from(matches!(route, Route::Open | Route::Continue)); // 201 Created
    let body = work(route, req, &segs, q, &mut state.lock().unwrap());
    let (status, body) = body.map_or_else(|e| (e, error(e)), |b| (ok, b));
    (!inject.drop_after_commit).then_some((status, inject.retry_after, body.to_string()))
}

/// The normal answer of `route`, or the status of a missing or malformed field (`400`).
fn work(r: Route, req: &Request, segs: &[&str], q: &str, s: &mut State) -> Result<Value, u16> {
    let (stop, left) = (s.stop, if s.stop { 0 } else { SECONDS_LEFT });
    let h = |name: &str| req.2.get(name).map(String::as_str).ok_or(400u16);
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
            e["status"] = if stop && sealed(&e) { "sealed" } else { "open" }.into();
            (e["sealed"], e["stop"]) = (sealed(&e).into(), stop.into());
            e
        }
        Route::Chunk => {
            let offset: u64 = segs[4].parse().map_err(|_| 400u16)?;
            let (len, last) = (num("goalcup-length")?, num("goalcup-last-seq")?);
            let (id, head) = (h("goalcup-entry-id")?, h("goalcup-head")?);
            s.chunks.push(Chunk {
                entry_id: id.into(),
                offset,
                body: req.3.clone(),
            });
            let e = entry(&mut s.entries, id);
            e["offset"] = (e["offset"].as_u64().unwrap_or(0) + len).into();
            (e["next_seq"], e["head"]) = ((last + 1).into(), head.into());
            json!({"offset": e["offset"], "next_seq": e["next_seq"], "head": e["head"],
                "received_at": "2026-11-06T10:00:01Z", "sealed": sealed(e)})
        }
        Route::Heartbeat => {
            let mut beat: Value = serde_json::from_slice(&req.3).map_err(|_| 400u16)?;
            let id = beat["entry_id"].as_str().ok_or(400u16)?.to_string();
            let c = &beat["counts"];
            let deep = |m: &Map<String, Value>| m.values().any(|v| v.is_object() || v.is_array());
            if !c.is_null() && c.as_object().is_none_or(deep) {
                let n = c.to_string().len(); // dropped from the kept copy, never refused
                (beat["counts"], beat["counts_dropped"]) = (Value::Null, n.into());
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
