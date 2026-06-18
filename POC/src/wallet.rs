//! Load signer wallets from a `.wallet.toml` so secrets never pass through CLI
//! args, environment variables, or shell history.
//!
//! Expected format (repo-root `.wallet.toml`):
//! ```toml
//! [[wallet]]
//! name = "person-1"
//! secret-phrase = "word word ... word"
//! ss58-address = "5C7Vxn…"
//! ```

use anyhow::{ensure, Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct WalletFile {
    wallet: Vec<WalletEntry>,
}

#[derive(Deserialize, Clone)]
pub struct WalletEntry {
    // Only `secret-phrase` is required; the rest are informational, so keep them
    // optional to tolerate typos/variations in the wallet file.
    #[serde(default)]
    pub name: String,
    #[serde(rename = "secret-phrase")]
    pub secret_phrase: String,
    #[serde(rename = "ss58-address", default)]
    pub ss58_address: Option<String>,
}

/// Parse the wallet file and return its entries (order preserved).
pub fn load(path: &str) -> Result<Vec<WalletEntry>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading wallet file {path}"))?;
    let parsed: WalletFile =
        toml::from_str(&text).with_context(|| format!("parsing wallet file {path}"))?;
    ensure!(!parsed.wallet.is_empty(), "no [[wallet]] entries in {path}");
    Ok(parsed.wallet)
}
