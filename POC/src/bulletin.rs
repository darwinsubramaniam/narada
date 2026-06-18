//! Bulletin Chain side: store call bytes, reconstruct the CID from a digest, and
//! fetch bytes back from the public IPFS gateway.
//!
//! Bulletin's `transactionStorage.store` content-addresses with `Blake2b256` + the
//! `raw` codec by default, so the content hash == `blake2_256(data)` == the multisig
//! call hash. That equivalence is why the executor can rebuild the CID purely from
//! the on-chain call hash.

use anyhow::{anyhow, Context, Result};
use cid::Cid;

/// IPFS multicodec for `raw` leaf data.
const RAW_CODEC: u64 = 0x55;
/// IPFS multihash code for `blake2b-256`.
const BLAKE2B_256: u64 = 0xb220;

/// Build the CIDv1 (`bafk…`) for a 32-byte blake2b-256 digest — i.e. for the
/// multisig call hash. Matches the hex form `0x0155a0e402<digest>` the docs show.
pub fn cid_from_digest(digest: &[u8; 32]) -> Result<Cid> {
    let mh = cid::multihash::Multihash::<64>::wrap(BLAKE2B_256, digest)
        .map_err(|e| anyhow!("multihash wrap failed: {e}"))?;
    Ok(Cid::new_v1(RAW_CODEC, mh))
}

/// `GET {gateway}/ipfs/<CID>?format=raw` — the exact raw block someone stored on
/// Bulletin. `?format=raw` is what the Bulletin Console uses for raw-codec blocks,
/// returning the stored bytes verbatim (no UnixFS interpretation).
pub async fn fetch_from_gateway(gateway: &str, cid: &Cid) -> Result<Vec<u8>> {
    let url = format!("{gateway}/ipfs/{cid}?format=raw");
    let resp = reqwest::get(&url)
        .await
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()
        .with_context(|| format!("gateway returned an error for {url}"))?;
    Ok(resp.bytes().await?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cid_matches_documented_byte_layout() {
        let digest = [0xABu8; 32];
        let cid = cid_from_digest(&digest).unwrap();
        // CIDv1 = <version 0x01><raw 0x55><mh-code 0xb220 varint a0 e4 02><len 0x20><digest>,
        // i.e. the `0x0155a0e402…` hex form the Polkadot docs show.
        let mut expected = vec![0x01, 0x55, 0xa0, 0xe4, 0x02, 0x20];
        expected.extend_from_slice(&digest);
        assert_eq!(cid.to_bytes(), expected);
        assert!(cid.to_string().starts_with("bafk"), "expected base32 CIDv1 'bafk…'");
    }
}
