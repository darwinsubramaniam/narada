//! The subcommands that make up the demonstration.
//!
//! The load-bearing claim of this POC: in `execute`, signatory B obtains BOTH the
//! pending operation (the call hash) AND the call data with **nothing handed over
//! from A** — the hash is discovered by iterating the on-chain `Multisig.Multisigs`
//! map under the multisig account, and the call bytes come solely from the Bulletin
//! Chain. The only shared knowledge is the multisig membership (public config).
//!
//! The multisig lives on **Paseo Asset Hub** (post-migration home of multisig +
//! balances); call data lives on the **Paseo Bulletin Chain**.

use anyhow::{anyhow, ensure, Context, Result};
use std::str::FromStr;
use subxt::utils::{AccountId32, MultiAddress};
use subxt_signer::{sr25519::Keypair, SecretUri};

use crate::bulletin::{cid_from_digest, fetch_from_gateway};
use crate::chains::{self, asset_hub, IPFS_GATEWAY};
use crate::multisig::{blake2_256, multi_account_id, other_signatories};

const THRESHOLD: u16 = 2;

type RuntimeCall = asset_hub::runtime_types::asset_hub_paseo_runtime::RuntimeCall;
type Weight = asset_hub::runtime_types::sp_weights::weight_v2::Weight;
type Timepoint = asset_hub::runtime_types::pallet_multisig::Timepoint<u32>;

fn keypair(seed: &str) -> Result<Keypair> {
    let uri = SecretUri::from_str(seed).context("parsing signer seed")?;
    Keypair::from_uri(&uri).map_err(|e| anyhow!("building keypair: {e}"))
}

fn account_of(kp: &Keypair) -> AccountId32 {
    AccountId32(kp.public_key().0)
}

/// Print the derived 2-of-2 multisig account for the A/B set.
pub fn address(a_seed: &str, b_seed: &str) -> Result<()> {
    let a = account_of(&keypair(a_seed)?);
    let b = account_of(&keypair(b_seed)?);
    let multisig = multi_account_id(&[a.clone(), b.clone()], THRESHOLD);
    println!("signatory A : {a}");
    println!("signatory B : {b}");
    println!("multisig    : {multisig}  (threshold {THRESHOLD})");
    println!("\nFund A, B, and the multisig account with PAS on Paseo Asset Hub, and");
    println!("authorize A on the Bulletin Chain Console storage faucet:");
    println!("  https://paritytech.github.io/polkadot-bulletin-chain/");
    Ok(())
}

/// Send `pas_each` PAS from BOTH signatories to the derived multisig account so it
/// has funds for the demonstrated transfer.
pub async fn fund_multisig(a_seed: &str, b_seed: &str, pas_each: u128) -> Result<()> {
    let a = keypair(a_seed)?;
    let b = keypair(b_seed)?;
    let multisig = multi_account_id(&[account_of(&a), account_of(&b)], THRESHOLD);
    // 1 PAS = 1e10 plancks on Asset Hub.
    let amount = pas_each.checked_mul(10_000_000_000).context("amount overflow")?;

    let ah_cli = chains::asset_hub_client().await?;
    for (label, signer) in [("person-1 (A)", &a), ("person-2 (B)", &b)] {
        println!("sending {pas_each} PAS from {label} → multisig {multisig}…");
        let tx = asset_hub::tx()
            .balances()
            .transfer_keep_alive(MultiAddress::Id(multisig.clone()), amount);
        ah_cli
            .at_current_block()
            .await?
            .transactions()
            .sign_and_submit_then_watch_default(&tx, signer)
            .await
            .context("submitting fund transfer")?
            .wait_for_finalized_success()
            .await
            .context("fund transfer did not finalize")?;
        println!("  ✅ funded");
    }
    println!("multisig {multisig} now funded with {} PAS total.", pas_each * 2);
    Ok(())
}

/// Signatory A: store the call on Bulletin, then `approve_as_multi` on Asset Hub.
/// With `memo`, the call becomes `batch_all([transfer, remark_with_event(memo)])` so
/// two otherwise-identical payments (same payee + amount, different invoice) get
/// DISTINCT call hashes — and the memo is an on-chain audit reference.
pub async fn init(
    a_seed: &str,
    b_seed: &str,
    recipient: Option<String>,
    amount: u128,
    memo: Option<String>,
) -> Result<()> {
    let a = keypair(a_seed)?;
    let b = keypair(b_seed)?;
    let a_acct = account_of(&a);
    let b_acct = account_of(&b);
    let signatories = vec![a_acct.clone(), b_acct.clone()];
    let multisig = multi_account_id(&signatories, THRESHOLD);
    let dest = match recipient {
        Some(s) => AccountId32::from_str(&s).map_err(|e| anyhow!("bad recipient SS58: {e}"))?,
        None => b_acct.clone(),
    };

    // Connect Asset Hub and encode the exact call bytes (= SCALE(RuntimeCall)) that
    // we hash, store, and later decode. `call_data` gives us those bytes.
    let ah_cli = chains::asset_hub_client().await?;
    let ah_at = ah_cli.at_current_block().await?;
    let call_bytes = if let Some(m) = &memo {
        // batch_all([transfer, remark]) — distinct hash per invoice, atomic execution.
        let transfer_rc = RuntimeCall::Balances(
            asset_hub::runtime_types::pallet_balances::pallet::Call::transfer_keep_alive {
                dest: MultiAddress::Id(dest.clone()),
                value: amount,
            },
        );
        let remark_rc = RuntimeCall::System(
            asset_hub::runtime_types::frame_system::pallet::Call::remark_with_event {
                remark: m.clone().into_bytes(),
            },
        );
        let batch = asset_hub::tx()
            .utility()
            .batch_all(vec![transfer_rc, remark_rc]);
        ah_at
            .transactions()
            .call_data(&batch)
            .context("encoding batch call data")?
    } else {
        let transfer = asset_hub::tx()
            .balances()
            .transfer_keep_alive(MultiAddress::Id(dest.clone()), amount);
        ah_at
            .transactions()
            .call_data(&transfer)
            .context("encoding call data")?
    };
    let call_hash = blake2_256(&call_bytes);
    let cid = cid_from_digest(&call_hash)?;

    // Guard: if this EXACT call is already queued, a second approve_as_multi(None)
    // would fail with `Multisig::NoTimepoint`. To queue several transactions (the
    // accountant-prepares-many pattern), use DISTINCT --recipient/--amount so each
    // call has a distinct hash.
    {
        let storage = ah_at
            .storage()
            .entry(asset_hub::storage().multisig().multisigs())?;
        let mut iter = storage.iter((multisig.clone(),)).await?;
        while let Some(item) = iter.next().await {
            let item = item?;
            let (_acct, h): (AccountId32, [u8; 32]) = item.key()?.decode()?;
            if h == call_hash {
                let ms = item.value().decode()?;
                println!(
                    "⚠️ this exact call is already queued (timepoint height {}, index {}).",
                    ms.when.height, ms.when.index
                );
                println!("   Run `execute` to process it, or queue a DISTINCT transaction");
                println!("   with a different --recipient/--amount.");
                return Ok(());
            }
        }
    }

    // 1) Store the SCALE-encoded call on the Bulletin Chain (feeless; A must be
    //    authorized via the storage faucet first).
    println!("storing {} call bytes on the Bulletin Chain…", call_bytes.len());
    let bulletin_cli = chains::bulletin_client().await?;
    let bulletin_at = bulletin_cli.at_current_block().await?;
    let store = chains::bulletin::tx()
        .transaction_storage()
        .store(call_bytes.clone());
    bulletin_at
        .transactions()
        .sign_and_submit_then_watch_default(&store, &a)
        .await
        .context("submitting transactionStorage.store (is A authorized on Bulletin?)")?
        .wait_for_finalized_success()
        .await
        .context("transactionStorage.store did not finalize successfully")?;
    println!("  stored. content CID = {cid}");

    // 2) Register A's approval on Asset Hub with only the call hash (max_weight is
    //    irrelevant for approve — it never executes — so pass zero). Use a FRESH
    //    at-block: the Bulletin store can take tens of seconds to finalize, so the
    //    block pinned earlier would be stale for nonce/mortality.
    println!("submitting approve_as_multi on Paseo Asset Hub…");
    let approve = asset_hub::tx().multisig().approve_as_multi(
        THRESHOLD,
        other_signatories(&signatories, &a_acct),
        None,
        call_hash,
        Weight { ref_time: 0, proof_size: 0 },
    );
    ah_cli
        .at_current_block()
        .await?
        .transactions()
        .sign_and_submit_then_watch_default(&approve, &a)
        .await
        .context("submitting approve_as_multi")?
        .wait_for_finalized_success()
        .await
        .context("approve_as_multi did not finalize successfully")?;

    println!("\n✅ init complete");
    println!("multisig account : {multisig}");
    println!("call hash        : 0x{}", hex::encode(call_hash));
    println!("bulletin CID     : {cid}");
    println!("\nNow run, as signatory B — no hash needed, it's discovered on-chain:");
    println!("  narada-poc execute");
    Ok(())
}

/// Read-only: list the multisig's pending operations, decoding each queued call
/// from Bulletin so a reviewer ("management") can see WHAT they're approving.
pub async fn pending(a_seed: &str, b_seed: &str) -> Result<()> {
    let a = account_of(&keypair(a_seed)?);
    let b = account_of(&keypair(b_seed)?);
    let multisig = multi_account_id(&[a, b], THRESHOLD);

    let ah_cli = chains::asset_hub_client().await?;
    let ah_at = ah_cli.at_current_block().await?;
    let storage = ah_at
        .storage()
        .entry(asset_hub::storage().multisig().multisigs())?;
    let mut iter = storage.iter((multisig.clone(),)).await?;

    println!("pending operations on multisig {multisig}:");
    let mut n = 0;
    while let Some(item) = iter.next().await {
        let item = item?;
        let (_acct, call_hash): (AccountId32, [u8; 32]) = item.key()?.decode()?;
        let ms = item.value().decode()?;
        n += 1;
        println!("\n[{n}] call hash 0x{}", hex::encode(call_hash));
        println!(
            "    timepoint : height {}, index {}",
            ms.when.height, ms.when.index
        );
        println!("    approvals : {}", ms.approvals.0.len());
        // Fetch + decode the call from Bulletin so the reviewer sees its intent.
        let cid = cid_from_digest(&call_hash)?;
        match fetch_from_gateway(IPFS_GATEWAY, &cid).await {
            Ok(bytes) if blake2_256(&bytes) == call_hash => {
                match decode_runtime_call(&bytes, ah_at.metadata_ref()) {
                    Ok(call) => {
                        if let Some(memo) = extract_remark(&call) {
                            println!("    invoice   : \"{memo}\"");
                        }
                        println!("    call      : {call:?}");
                    }
                    Err(e) => println!("    call      : <decode failed: {e}>"),
                }
            }
            Ok(_) => println!("    call      : <bulletin bytes hash mismatch>"),
            Err(e) => println!("    call      : <not retrievable from Bulletin: {e}>"),
        }
    }
    if n == 0 {
        println!("  (none)");
    }
    Ok(())
}

/// Signatory B: discover pending multisig operations ON-CHAIN, fetch each call's
/// bytes from Bulletin, verify, and `as_multi` to execute. Nothing is handed over
/// from A — only the multisig membership (public config) is shared.
pub async fn execute(a_seed: &str, b_seed: &str, call_hash_filter: Option<&str>) -> Result<()> {
    let a = keypair(a_seed)?;
    let b = keypair(b_seed)?;
    let a_acct = account_of(&a);
    let b_acct = account_of(&b);
    let signatories = vec![a_acct.clone(), b_acct.clone()];
    let multisig = multi_account_id(&signatories, THRESHOLD);
    let want = match call_hash_filter {
        Some(s) => Some(parse_hash(s)?),
        None => None,
    };

    let ah_cli = chains::asset_hub_client().await?;
    let ah_at = ah_cli.at_current_block().await?;

    // Discover pending operations by iterating Multisig.Multisigs with the FIRST key
    // fixed to the multisig account — the call hash is the SECOND key, read off-chain
    // from nobody, straight from chain state.
    let storage = ah_at
        .storage()
        .entry(asset_hub::storage().multisig().multisigs())
        .context("building Multisig.Multisigs address")?;
    let mut iter = storage
        .iter((multisig.clone(),))
        .await
        .context("iterating Multisig.Multisigs for the multisig account")?;

    // Timepoint isn't Clone, so keep its Copy fields and rebuild it per operation.
    let mut pending: Vec<([u8; 32], u32, u32)> = Vec::new();
    while let Some(item) = iter.next().await {
        let item = item?;
        let (_acct, call_hash): (AccountId32, [u8; 32]) = item.key()?.decode()?;
        if let Some(w) = want {
            if w != call_hash {
                continue;
            }
        }
        let ms = item.value().decode()?;
        pending.push((call_hash, ms.when.height, ms.when.index));
    }
    ensure!(
        !pending.is_empty(),
        "no pending multisig operations found on {multisig}"
    );
    println!(
        "discovered {} pending operation(s) on multisig {multisig} (from chain — no hash supplied)",
        pending.len()
    );

    for (call_hash, height, index) in pending {
        println!(
            "\n— call hash 0x{} (timepoint height {height}, index {index})",
            hex::encode(call_hash)
        );

        // Fetch the call DATA from the Bulletin Chain — the only place B gets it.
        let cid = cid_from_digest(&call_hash)?;
        println!("  fetching call data from Bulletin: {IPFS_GATEWAY}/ipfs/{cid}");
        let fetched = fetch_from_gateway(IPFS_GATEWAY, &cid).await?;

        // Integrity gate: the bytes must hash to the on-chain call hash.
        let recomputed = blake2_256(&fetched);
        ensure!(
            recomputed == call_hash,
            "hash mismatch: bulletin bytes hash to 0x{} but chain key is 0x{}",
            hex::encode(recomputed),
            hex::encode(call_hash),
        );
        println!("  integrity OK: blake2_256(fetched) == on-chain call hash");

        // Decode into the typed RuntimeCall (metadata-aware) and execute.
        let call = decode_runtime_call(&fetched, ah_at.metadata_ref())?;
        let timepoint = Timepoint { height, index };
        let as_multi = asset_hub::tx().multisig().as_multi(
            THRESHOLD,
            other_signatories(&signatories, &b_acct),
            Some(timepoint),
            call,
            // Generous max_weight (refunded down to actual) — covers a batch with a
            // remark, not just a bare transfer.
            Weight { ref_time: 5_000_000_000, proof_size: 500_000 },
        );
        println!("  submitting as_multi on Paseo Asset Hub to execute…");
        let events = ah_cli
            .at_current_block()
            .await?
            .transactions()
            .sign_and_submit_then_watch_default(&as_multi, &b)
            .await
            .context("submitting as_multi")?
            .wait_for_finalized_success()
            .await
            .context("as_multi did not finalize successfully")?;

        match events.find_first::<asset_hub::multisig::events::MultisigExecuted>() {
            Some(Ok(ev)) => match ev.result {
                Ok(()) => println!("  ✅ executed — inner call dispatched OK (transfer moved funds)."),
                Err(e) => println!(
                    "  ⚠️ executed but the inner call FAILED: {e:?} (is the multisig funded?)"
                ),
            },
            Some(Err(_)) | None => println!("  ✅ executed (no MultisigExecuted event decoded)."),
        }
    }

    println!(
        "\n✅ execute complete — call data discovered & fetched purely from chain + Bulletin \
         (nothing passed from A to B)."
    );
    Ok(())
}

/// If the call is a `batch_all` containing a `remark_with_event`, return the remark
/// text (the invoice reference) so it can be shown to a reviewer.
fn extract_remark(call: &RuntimeCall) -> Option<String> {
    use asset_hub::runtime_types::frame_system::pallet::Call as SystemCall;
    use asset_hub::runtime_types::pallet_utility::pallet::Call as UtilityCall;
    if let RuntimeCall::Utility(UtilityCall::batch_all { calls }) = call {
        for inner in calls {
            if let RuntimeCall::System(SystemCall::remark_with_event { remark }) = inner {
                return Some(String::from_utf8_lossy(remark).into_owned());
            }
        }
    }
    None
}

/// Decode SCALE call bytes into the generated `RuntimeCall` using the chain's
/// type registry (generated types impl `DecodeAsType`, not plain `Decode`).
fn decode_runtime_call(bytes: &[u8], metadata: &subxt::Metadata) -> Result<RuntimeCall> {
    use subxt::ext::scale_decode::DecodeAsType;
    let types = metadata.types();
    let type_id = types
        .types
        .iter()
        .find(|t| {
            let segs = &t.ty.path.segments;
            segs.last().map(String::as_str) == Some("RuntimeCall")
                && segs.iter().any(|s| s == "asset_hub_paseo_runtime")
        })
        .map(|t| t.id)
        .ok_or_else(|| anyhow!("RuntimeCall type not found in metadata"))?;
    let mut cursor = bytes;
    let call = RuntimeCall::decode_as_type(&mut cursor, type_id, types)
        .map_err(|e| anyhow!("decoding RuntimeCall: {e}"))?;
    ensure!(cursor.is_empty(), "trailing bytes after decoding the call");
    Ok(call)
}

fn parse_hash(s: &str) -> Result<[u8; 32]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s).context("call hash is not valid hex")?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("call hash must be 32 bytes"))
}
