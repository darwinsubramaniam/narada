//! Multisig primitives that must byte-for-byte match `pallet-multisig`:
//! the account derivation and the call hash (`blake2_256` of the SCALE-encoded
//! outer `RuntimeCall`). Getting these exactly right is what lets the executor
//! look up the on-chain pending multisig and fetch the matching bytes from Bulletin.

use subxt::ext::codec::Encode;
use subxt::utils::AccountId32;

/// BLAKE2b with a 32-byte digest — identical to substrate's `blake2_256` and to
/// the Bulletin Chain's default content hash. The whole POC hinges on this being
/// the *same* function on both sides.
pub fn blake2_256(data: &[u8]) -> [u8; 32] {
    use blake2::digest::{consts::U32, Digest};
    let mut hasher = blake2::Blake2b::<U32>::new();
    hasher.update(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    out
}

/// Replicates `pallet_multisig::Pallet::multi_account_id`:
/// `blake2_256(("modlpy/utilisuba", sorted(signatories), threshold).encode())`.
/// `signatories` must be the FULL set (including every party); it is sorted here.
pub fn multi_account_id(signatories: &[AccountId32], threshold: u16) -> AccountId32 {
    let mut who = signatories.to_vec();
    who.sort_by(|a, b| a.0.cmp(&b.0));
    let prefix: [u8; 16] = *b"modlpy/utilisuba";
    let entropy = (prefix, who, threshold).encode();
    AccountId32(blake2_256(&entropy))
}

/// The `other_signatories` argument for `signer`'s multisig extrinsics: the full
/// set minus the caller, sorted (as the pallet requires).
pub fn other_signatories(all: &[AccountId32], signer: &AccountId32) -> Vec<AccountId32> {
    let mut others: Vec<AccountId32> = all
        .iter()
        .filter(|a| a.0 != signer.0)
        .cloned()
        .collect();
    others.sort_by(|a, b| a.0.cmp(&b.0));
    others
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(hex_str: &str) -> AccountId32 {
        let bytes = hex::decode(hex_str).unwrap();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        AccountId32(arr)
    }

    /// Reference value independently computed (Python `hashlib.blake2b`) for the
    /// well-known //Alice + //Bob sr25519 dev accounts at threshold 2. Non-circular:
    /// it does not reuse this module's implementation.
    #[test]
    fn multi_account_id_matches_independent_reference() {
        let alice = id("d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d");
        let bob = id("8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48");
        let expected = id("83b70134afe83e035d51b9b6543bae58fc4ad7495df986b619e71b2581bf6ec5");

        assert_eq!(multi_account_id(&[alice.clone(), bob.clone()], 2), expected);
        // Derivation must be order-independent (signatories are sorted internally).
        assert_eq!(multi_account_id(&[bob.clone(), alice.clone()], 2), expected);
        // Threshold is part of the entropy, so it must change the account.
        assert_ne!(multi_account_id(&[alice, bob], 3), expected);
    }
}
