// ------------------------------------------------------------------------ upload and entries

use super::*;

/// `goalcup upload` when there is no run on this computer.
pub fn upload_no_run() -> String {
    "Run goalcup start first: no run has been started from this machine, so there is nothing to \
     upload. To send one part by its id: goalcup upload <part-id>."
        .to_string()
}

/// `goalcup upload` when the newest run was never opened on the server.
pub fn upload_local_run(run_id: &str) -> String {
    format!(
        "Name a part of another run: goalcup upload <part-id>. Nothing to upload: the newest run \
         ({run_id}) \
         is local, so it was recorded on this computer only and never opened on the server."
    )
}

/// `goalcup upload` when the newest run is all on the server.
pub fn upload_nothing_left(run_id: &str) -> String {
    format!("Nothing left to send: run {run_id} is fully uploaded. {NOTHING_MORE}")
}

/// A part id that this computer did not open on a server.
pub fn upload_not_opened(entry_id: &str) -> String {
    format!(
        "Name a part of a run started on this computer: goalcup upload <part-id>. Part {entry_id} \
         was never opened on the server from here, so there is nothing to upload."
    )
}

/// The run's credential is not on this computer.
pub fn upload_not_here(entry_id: &str) -> String {
    format!(
        "Run goalcup upload on the computer that started the run: this run was not started on \
         this machine (part {entry_id} has no saved credential here), and only the machine that \
         started a run can upload it."
    )
}

/// The server is away; goalcup waits `wait_secs` and asks again.
pub fn upload_waiting(why: &str, wait_secs: u64) -> String {
    format!(
        "Waiting for the server ({why}). Trying again in {wait_secs}s. Keep this terminal open, \
         or press Ctrl-C and run goalcup upload later: it carries on from where it stopped."
    )
}

/// The server refused the upload for good.
pub fn upload_refused(server: &str, entry_id: &str, why: &str) -> String {
    format!(
        "Run goalcup upload again. If it is refused again, write to {SUPPORT_ADDRESS}. The server \
         at {server} refused part {entry_id} of your run: {why}"
    )
}

/// Part of the run did not get through.
pub fn upload_incomplete(entry_id: &str, bytes: u64, why: &str) -> String {
    format!(
        "Check the network, then run goalcup upload again. Part {entry_id} of your run is not \
         fully uploaded ({bytes} bytes are): {why}. Nothing is lost; it waits on this computer."
    )
}

/// The upload finished, and the run is sealed.
pub fn upload_finished() -> String {
    format!("Uploaded the rest of your run. {NOTHING_MORE}")
}

/// The upload finished, and the part is still recording: its own goalcup sends the rest.
pub fn upload_not_sealed() -> String {
    "Nothing to do: this part of the run is still recording, and what exists so far is uploaded. \
     The rest goes up by itself."
        .to_string()
}

/// The upload finished, and nothing records the part any more, but it was never sealed (its
/// goalcup stopped first, or its seal could not be written): only `goalcup seal` seals it.
pub fn upload_seal_missing(entry_id: &str) -> String {
    format!(
        "Run goalcup seal {entry_id}, then goalcup upload. This part of the run is uploaded, but \
         it was not sealed. If the disk is full, free some space first."
    )
}

/// The first lines of `goalcup entries`.
pub fn entries_intro(none: bool) -> Vec<String> {
    if none {
        vec![
            "To start a run: goalcup prep, then goalcup start --harness <id> --practice."
                .to_string(),
            "To show the newest run you started here, even a finished one: goalcup status"
                .to_string(),
            String::new(),
            "Parts of runs on this computer:".to_string(),
        ]
    } else {
        vec![
            "To show one part in full, run: goalcup status <part-id>".to_string(),
            String::new(),
            "Parts of runs on this computer (part id, harness, state, folder):".to_string(),
        ]
    }
}
