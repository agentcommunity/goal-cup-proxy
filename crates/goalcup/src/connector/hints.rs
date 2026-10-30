//! Host hints and refusals read from the harness's config files and named variables (PRD-11
//! sections 2 and 3). The hints slice fills both functions in; until then they find nothing.

use std::path::Path;

use super::{Connector, Env, Host, Refusal};

/// Hosts the connector's `host_hints` keys and `hint_env` variables name (the `config` list), and
/// one `local_model` note per name that points at this computer.
pub(crate) fn collect(
    _connector: &Connector,
    _env: &Env,
    _home: &Path,
) -> (Vec<Host>, Vec<serde_json::Value>) {
    (Vec::new(), Vec::new())
}

/// The first `refuse_if_set` key that holds a value: file and key, never the value.
pub(crate) fn refusal(_connector: &Connector, _env: &Env, _home: &Path) -> Option<Refusal> {
    None
}
