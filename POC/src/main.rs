//! narada POC — proving the Polkadot Bulletin Chain can carry multisig call data
//! so signatories no longer have to sync it off-chain.
//!
//! Flow: `init` (signatory A) stores the call on Bulletin + approves on Asset Hub;
//! `execute` (signatory B) fetches the call from Bulletin by its on-chain hash,
//! verifies, and executes the multisig. See README.md.
//!
//! Signers default to the first two wallets in the repo-root `.wallet.toml`
//! (A = wallet[0], B = wallet[1]); override with `--a-seed` / `--b-seed`.

mod bulletin;
mod chains;
mod commands;
mod multisig;
mod wallet;

use anyhow::{ensure, Result};
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "narada-poc", about, version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Signer selection shared by every subcommand.
#[derive(Args)]
struct Signers {
    /// Path to the wallet TOML (defaults to the repo-root `.wallet.toml`).
    #[arg(long, default_value = "../.wallet.toml")]
    wallets: String,
    /// Override signatory A's seed/phrase (else the first wallet).
    #[arg(long, env = "SIGNER_A_SEED")]
    a_seed: Option<String>,
    /// Override signatory B's seed/phrase (else the second wallet).
    #[arg(long, env = "SIGNER_B_SEED")]
    b_seed: Option<String>,
}

impl Signers {
    /// Resolve (A, B) seeds, reading the wallet file only when needed.
    fn resolve(&self) -> Result<(String, String)> {
        if let (Some(a), Some(b)) = (self.a_seed.clone(), self.b_seed.clone()) {
            return Ok((a, b));
        }
        let wallets = wallet::load(&self.wallets)?;
        ensure!(
            wallets.len() >= 2,
            "need at least 2 wallets in {} (found {})",
            self.wallets,
            wallets.len()
        );
        eprintln!(
            "signers from {}: A = {} ({}), B = {} ({})",
            self.wallets,
            wallets[0].name,
            wallets[0].ss58_address.as_deref().unwrap_or("?"),
            wallets[1].name,
            wallets[1].ss58_address.as_deref().unwrap_or("?"),
        );
        let a = self
            .a_seed
            .clone()
            .unwrap_or_else(|| wallets[0].secret_phrase.clone());
        let b = self
            .b_seed
            .clone()
            .unwrap_or_else(|| wallets[1].secret_phrase.clone());
        Ok((a, b))
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the derived 2-of-2 multisig account (and funding/auth reminders).
    Address {
        #[command(flatten)]
        signers: Signers,
    },
    /// Signatory A: store the call on Bulletin, then approve_as_multi on Asset Hub.
    Init {
        #[command(flatten)]
        signers: Signers,
        /// Transfer recipient (SS58). Defaults to signatory B.
        #[arg(long)]
        recipient: Option<String>,
        /// Transfer amount in plancks (1 PAS = 1e10).
        #[arg(long, default_value_t = 1_000_000_000)]
        amount: u128,
    },
    /// Read-only: list the multisig's pending operations with their decoded calls.
    Pending {
        #[command(flatten)]
        signers: Signers,
    },
    /// Signatory B: discover pending multisig ops on-chain, fetch each call from
    /// Bulletin, verify, and execute. No hash needed.
    Execute {
        #[command(flatten)]
        signers: Signers,
        /// Optional: only process this specific call hash (else all pending ops).
        #[arg(long)]
        call_hash: Option<String>,
    },
    /// Send PAS from both signatories to the multisig account (so the demo
    /// transfer has funds to move).
    FundMultisig {
        #[command(flatten)]
        signers: Signers,
        /// PAS to send from EACH signatory to the multisig.
        #[arg(long, default_value_t = 100)]
        pas_each: u128,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Address { signers } => {
            let (a, b) = signers.resolve()?;
            commands::address(&a, &b)
        }
        Cmd::Init { signers, recipient, amount } => {
            let (a, b) = signers.resolve()?;
            commands::init(&a, &b, recipient, amount).await
        }
        Cmd::Pending { signers } => {
            let (a, b) = signers.resolve()?;
            commands::pending(&a, &b).await
        }
        Cmd::Execute { signers, call_hash } => {
            let (a, b) = signers.resolve()?;
            commands::execute(&a, &b, call_hash.as_deref()).await
        }
        Cmd::FundMultisig { signers, pas_each } => {
            let (a, b) = signers.resolve()?;
            commands::fund_multisig(&a, &b, pas_each).await
        }
    }
}
