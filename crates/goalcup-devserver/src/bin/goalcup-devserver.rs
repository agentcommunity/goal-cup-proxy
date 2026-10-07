//! `goalcup-devserver --addr 127.0.0.1:0 [--key-seed <hex>] [--print-key]`
//!
//! Every path through this binary prints the banner to stderr first. There is no quiet
//! mode: an operator who cannot see what the key is worth is the one failure this crate
//! can actually cause.
#![forbid(unsafe_code)]

use clap::Parser;
use goalcup_devserver::{DEV_BANNER, DevServer, ReceiptAuthority, dev_router};
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(
    name = "goalcup-devserver",
    about = "DEVELOPMENT ONLY: a goal-cup server that signs receipts and acknowledgements"
)]
struct Args {
    /// Loopback address to listen on. Port 0 asks the OS for one, and the bound address is
    /// printed to stderr. **Loopback, or refused**: see [`loopback_only`].
    #[arg(long, default_value = "127.0.0.1:0")]
    addr: String,
    /// Sign with a 32-byte seed of your own, 64 hex characters, instead of the published
    /// placeholder. A trace signed this way needs `goalcup-verify --dev-key <hex>` and
    /// reads as `key_source: "supplied"` with the Flag `server_key_unpinned`, which `--dev`
    /// does not suppress.
    #[arg(long, value_name = "HEX")]
    key_seed: Option<String>,
    /// Receipt contract for this process. New-event mode also requires
    /// `--questionnaire-version`.
    #[arg(long, default_value = "legacy")]
    receipt_mode: String,
    /// Independently configured questionnaire contract identifier for new-event receipts.
    #[arg(long, value_name = "VERSION")]
    questionnaire_version: Option<String>,
    /// Print the public key to stdout and exit 0, without binding anything. What a script
    /// pins a daemon or a `--dev-key` against.
    #[arg(long)]
    print_key: bool,
    /// DEVELOPMENT ONLY: date every receipt and acknowledgement from this instant instead
    /// of the wall clock. RFC 3339 with a `Z`.
    ///
    /// A start receipt's `started_at` is this server's clock and a client may not name
    /// its own, so the window a receipt opens starts at the moment it was issued — and a
    /// daemon has already chained the arming records that CAUSED the receipt to be requested
    /// by then. Their `recorded_at` is therefore at or barely below `started_at`, and V13
    /// reports one second of rounding as `outside_window`. An end-to-end test needs the
    /// window to open unambiguously below the run, which is what `DevServer::set_clock` is
    /// for and what this exposes to a test driving the BINARY rather than the library.
    ///
    /// The clock does not run: every receipt and acknowledgement carries exactly this
    /// instant.
    #[arg(long, value_name = "RFC3339")]
    clock: Option<String>,
    /// DEVELOPMENT ONLY: select a strictly positive server-owned qualification window in
    /// seconds. Without this flag, old callers retain the default 168-hour window.
    #[arg(long, value_name = "SECONDS")]
    qualification_window_seconds: Option<u64>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    eprintln!("{DEV_BANNER}");

    // Before the key is derived and before `--print-key` can answer: an address this binary
    // will not serve on is a refusal whatever else was asked for, and a `--print-key` that
    // succeeded here would hand a script the key of a server that then refused to start.
    if let Err(message) = loopback_only(&args.addr) {
        eprintln!("--addr: {message}");
        return ExitCode::FAILURE;
    }

    let authority = match (
        args.receipt_mode.as_str(),
        args.questionnaire_version.as_deref(),
    ) {
        ("legacy", None) => ReceiptAuthority::Legacy,
        ("legacy", Some(_)) => {
            eprintln!("--questionnaire-version is valid only with --receipt-mode new-event");
            return ExitCode::FAILURE;
        }
        ("new-event", Some(version)) => match ReceiptAuthority::new_event(version) {
            Ok(authority) => authority,
            Err(e) => {
                eprintln!("--questionnaire-version: {e}");
                return ExitCode::FAILURE;
            }
        },
        ("new-event", None) => {
            eprintln!("--receipt-mode new-event requires --questionnaire-version");
            return ExitCode::FAILURE;
        }
        (mode, _) => {
            eprintln!("--receipt-mode: {mode:?} is not legacy or new-event");
            return ExitCode::FAILURE;
        }
    };

    let qualification_window_seconds = args.qualification_window_seconds;
    let server = match args.key_seed.as_deref() {
        None => match qualification_window_seconds {
            None => DevServer::with_authority(authority),
            Some(seconds) => DevServer::with_authority_and_qualification_window(authority, seconds),
        },
        Some(hex_seed) => match parse_seed(hex_seed) {
            Ok(seed) => match qualification_window_seconds {
                None => DevServer::with_seed_and_authority(seed, authority),
                Some(seconds) => DevServer::with_seed_and_authority_and_qualification_window(
                    seed, authority, seconds,
                ),
            },
            Err(message) => {
                eprintln!("--key-seed: {message}");
                return ExitCode::FAILURE;
            }
        },
    };
    let server = match server {
        Ok(server) => server,
        Err(e) => {
            if qualification_window_seconds.is_some() {
                eprintln!("--qualification-window-seconds: {e}");
            } else {
                eprintln!("receipt authority: {e}");
            }
            return ExitCode::FAILURE;
        }
    };
    eprintln!("{}", server.receipt_mode_banner());
    if let Some(seconds) = qualification_window_seconds {
        eprintln!("qualification window: {seconds} seconds (server-owned)");
    }
    let key = server.public_key_hex();
    // The banner names what the key IS, because "pinned" and "supplied" are the difference
    // between a verdict that means something and one that borrowed its trust.
    if args.key_seed.is_some() {
        eprintln!(
            "signing key {key} — from --key-seed. Nothing pins it: verify with \
             `goalcup-verify --dev-key {key}`, and expect `server_key_unpinned`."
        );
    } else {
        eprintln!(
            "signing key {key} — the PLACEHOLDER, whose seed is published in this \
             repository. Anybody can sign under it; verify with `goalcup-verify --dev`."
        );
    }
    if args.print_key {
        println!("{key}");
        return ExitCode::SUCCESS;
    }

    if let Some(at) = &args.clock {
        if goalcup_protocol::control::parse_z(at).is_none() {
            eprintln!(
                "--clock: {at:?} is not an instant this build can read \
                 (YYYY-MM-DDTHH:MM:SSZ)"
            );
            return ExitCode::FAILURE;
        }
        eprintln!("clock frozen at {at} — every receipt and acknowledgement is dated from it");
        server.set_clock(at);
    }

    let listener = match tokio::net::TcpListener::bind(&args.addr).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("could not bind {}: {e}", args.addr);
            return ExitCode::FAILURE;
        }
    };
    match listener.local_addr() {
        Ok(addr) => eprintln!("listening on http://{addr}"),
        Err(e) => eprintln!("bound, but could not read the local address: {e}"),
    }
    match axum::serve(listener, dev_router(Arc::new(server))).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("the server stopped: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Refuse an address that is not loopback.
///
/// This server has no authentication at all — deliberately, because it signs with a key
/// published in this repository and a token would protect nothing (see `router.rs`). What
/// makes that safe is that the socket is not reachable off the machine: a flag that said
/// "loopback address" while enforcing nothing would let `--addr 0.0.0.0:8080` expose
/// unauthenticated `receipt` and `chunks` to the network, where anybody could open an entry
/// under a run key of their choosing and be handed signatures for it.
/// `DaemonConfig::validate` refuses a non-loopback `control_addr` on the other side of the
/// same wire; this is the same refusal, on this side.
///
/// The string is parsed as a `SocketAddr` first, so a host NAME never reaches the bind: a
/// name resolves at connect time to whatever the resolver says today, which is not a fact
/// this check could hold. `localhost` is therefore refused by spelling — write `127.0.0.1`
/// or `[::1]` — and that is the intended reading rather than an oversight.
fn loopback_only(addr: &str) -> Result<(), String> {
    let parsed: std::net::SocketAddr = addr.parse().map_err(|e| {
        format!(
            "{addr:?} is not an <ip>:<port> address ({e}). Write the address out — a host \
             name resolves to whatever the resolver says at connect time, which is not \
             something this refusal could check."
        )
    })?;
    match parsed.ip().is_loopback() {
        true => Ok(()),
        false => Err(format!(
            "{addr} is not loopback. This server has no authentication — it signs with a key \
             published in this repository — so what keeps its receipt and chunk routes safe \
             is that the socket cannot be reached off this machine."
        )),
    }
}

/// 64 hex characters, decoded to the 32 bytes `DevServer::with_seed` takes. The one place
/// a key that is not the placeholder enters this crate.
fn parse_seed(hex_seed: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_seed).map_err(|e| format!("not hex: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| "a seed is 32 bytes, i.e. 64 hex characters".to_string())
}
