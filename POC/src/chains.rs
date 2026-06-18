//! Connections to the two chains the POC spans:
//! - **Paseo Asset Hub** — post-Asset-Hub-Migration home of `pallet-multisig`,
//!   `balances`, and governance. We run the multisig + `transfer_keep_alive` here
//!   (the relay chain's copies are being retired by the migration).
//! - **Paseo Bulletin** — content-addressed storage for the SCALE-encoded call bytes.
//!
//! Both runtimes are codegen'd from pinned metadata files under `metadata/`.

use subxt::{OnlineClient, PolkadotConfig};

/// Public Paseo Asset Hub RPC (multisig + balances + governance), paraId 1000.
pub const ASSET_HUB_RPC: &str = "wss://asset-hub-paseo-rpc.n.dwellir.com";

/// Public, Parity-hosted Paseo Bulletin Chain RPC. This is "Bulletin Paseo Next v2"
/// — the chain the Bulletin Console targets by default (`DEFAULT_NETWORK`), so it's
/// where storage-faucet authorizations land. (The older "Paseo Next" lives at
/// wss://paseo-bulletin-rpc.polkadot.io and is a *different* chain.)
pub const BULLETIN_RPC: &str = "wss://paseo-bulletin-next-rpc.polkadot.io";

/// Next v2 IPFS gateway — `GET {IPFS_GATEWAY}/ipfs/<CID>?format=raw` returns the bytes.
pub const IPFS_GATEWAY: &str = "https://paseo-bulletin-next-ipfs.polkadot.io";

#[subxt::subxt(runtime_metadata_path = "metadata/assethub.scale")]
pub mod asset_hub {}

#[subxt::subxt(runtime_metadata_path = "metadata/bulletin.scale")]
pub mod bulletin {}

/// Both chains are standard Substrate chains (AccountId32 + sr25519 + BlakeTwo256),
/// so `PolkadotConfig` works for each. subxt reads each chain's signed-extension set
/// from its metadata, so the same config drives both.
pub type Client = OnlineClient<PolkadotConfig>;

pub async fn asset_hub_client() -> anyhow::Result<Client> {
    Ok(OnlineClient::<PolkadotConfig>::from_url(ASSET_HUB_RPC).await?)
}

pub async fn bulletin_client() -> anyhow::Result<Client> {
    Ok(OnlineClient::<PolkadotConfig>::from_url(BULLETIN_RPC).await?)
}
