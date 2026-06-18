# narada POC — Bulletin-Chain-backed multisig call-data sync

Proves that the **Polkadot Bulletin Chain** can carry a multisig's **call data** so
signatories no longer have to share it off-chain (the manual, centralized step that
tools like Polkasafe/Mimir paper over today).

- **Multisig** runs on **Paseo Asset Hub** (`pallet-multisig` + `balances`). Asset
  Hub is the post-migration (AHM) home of these pallets, so the multisig and the
  `transfer_keep_alive` call both live here, not on the relay.
- **Call data** is stored on the **Paseo Bulletin Chain — "Paseo Next v2"** (the
  deployment the Bulletin Console targets by default; `transactionStorage`) and read
  back from its public IPFS gateway.

The linchpin: `pallet-multisig` keys pending calls by `blake2_256(SCALE(RuntimeCall))`,
and Bulletin's `store` content-addresses by the *same* Blake2b-256 hash. So the
executor rebuilds the storage CID directly from the on-chain call hash — no separate
mapping, and retrieval is self-verifying.

> 📐 **[ARCHITECTURE.md](./ARCHITECTURE.md)** — full research write-up: problem
> statement, data model, mermaid diagrams, and the live experiment results.

## What it does

| Subcommand | As | Action |
|---|---|---|
| `address` | — | Print the derived 2-of-2 multisig account + funding/auth reminders. |
| `fund-multisig` | — | Send PAS from both signatories to the multisig account (so the demo transfer has funds). |
| `init` | signatory A | Encode a `transfer_keep_alive` call (`--recipient`/`--amount`/`--memo` make it distinct) → `transactionStorage.store` on Bulletin → `approve_as_multi(call_hash)` on Asset Hub. With `--memo`, wraps the transfer + `remark_with_event` in `batch_all`. Refuses to double-queue an identical call. |
| `pending` | — (read-only) | List the multisig's queued operations, **decoding each call from Bulletin** so a reviewer sees what they're approving. |
| `execute` | signatory B | **Discover** pending ops on-chain (iterate `Multisig.Multisigs` by multisig account) → fetch each call's bytes from Bulletin by hash → assert `blake2_256(fetched) == on-chain call hash` → decode → `as_multi` to execute (`--call-hash` to pick one). |

`execute` is handed **nothing** from A: it discovers the pending operation's hash by
reading on-chain `Multisig.Multisigs`, and fetches the **call bytes** solely from the
Bulletin Chain. The only shared knowledge is the multisig membership (public config).
That is the whole point.

## Trust model & decodeability

Person B (the approver) can **always fully decode the call they're about to sign**, with
no centralized orchestration:

- **Fully decodeable, no blind-signing.** The bytes B fetches from Bulletin are the
  canonical SCALE encoding of the outer `RuntimeCall`. With the chain's type registry
  (its metadata), `DecodeAsType` reconstructs the *entire* call tree — nested
  `batch_all`, the inner transfer, the remark, everything (`pending` prints it). B then
  submits `as_multi` with that **decoded call**, not an opaque hash. And a call that
  *can't* be decoded also *can't* be executed (the pallet only runs a call whose bytes
  hash to the on-chain key), so "if it doesn't decode, don't sign" never blocks a
  legitimate payment.
- **Non-custodial.** There is **no central database** (the Polkasafe/Mimir model). The
  call data lives on a public chain, content-addressed, and is verified locally by
  `blake2_256(fetched) == on-chain call hash`. No party can withhold, alter, or forge it
  without breaking that check.
- **The hosted gateway is a convenience, not a dependency.** This POC fetches over the
  Parity-hosted HTTP gateway (`paseo-bulletin-next-ipfs.polkadot.io`) for simplicity,
  but the data is **peer-to-peer retrievable** — the Bulletin Chain serves it over IPFS
  Bitswap, and the official console prefers `p2p` for most networks. To remove the last
  central touchpoint, B fetches from their own Bulletin/IPFS node (or a light client)
  instead of the gateway; the bytes and the hash check are identical either way.

**Caveats on "always":**

- **Retention.** Bulletin keeps data ~14 days (`RetentionPeriod` = 201,600 blocks). A
  multisig left pending longer needs the storage **renewed** (`renew` /
  `enable_auto_renew`) or the bytes can be pruned — relevant for long-lived,
  governance-style queues.
- **Metadata version.** Decoding uses the chain's metadata; across a runtime upgrade a
  long-pending call should be decoded with version-matched metadata. This is a
  light-client/archive concern, not centralization — the metadata is part of the
  runtime, served by the chain itself.
- **Trustless reads.** Reading the call hash (Asset Hub) and the call data (Bulletin) can
  both go through a **light client** (smoldot) instead of a hosted RPC, so B need not
  trust any single provider.

## Prerequisites

1. **Rust** + `subxt-cli` 0.50 (`cargo install subxt-cli --version 0.50.0`).
2. Two signer wallets. By default the POC reads them from the repo-root
   `.wallet.toml` (A = first `[[wallet]]`, B = second) — run commands from the
   `POC/` directory so the default `../.wallet.toml` resolves, or pass
   `--wallets <path>`. `.wallet.toml` is git-ignored; never commit it.
   To use different keys ad hoc, override with `--a-seed` / `--b-seed` (a
   `//Derivation` URI or a mnemonic) or the `SIGNER_A_SEED` / `SIGNER_B_SEED` env vars.
3. **Fund** A, B, and the derived multisig account with PAS **on Paseo Asset Hub**
   (select Asset Hub in the [Paseo faucet](https://faucet.polkadot.io/)).
   Run `cargo run -- address` to print all three addresses.
4. **Authorize A on the Bulletin Chain** (one-time, ~14-day window) via the
   [Bulletin Chain Console](https://paritytech.github.io/polkadot-bulletin-chain/)
   → **Faucet → Storage Faucet → Authorize Account** (set transactions + bytes),
   signed with signatory **A**'s account (the first wallet) in a browser wallet.
   The Console defaults to "Paseo Next v2" — the chain this POC targets; make sure the
   selected network matches. `store` is feeless, so A needs no Bulletin tokens — only
   this authorization. (Storage authorization cannot be obtained programmatically; the
   faucet is the only path.)

## Run

```bash
cargo test                  # unit tests: multisig derivation + CID layout
cargo run -- address        # show A, B, and the multisig account
cargo run -- fund-multisig  # (once) send 100 PAS from A & B to the multisig
cargo run -- init           # signatory A: store on Bulletin + approve on Asset Hub
cargo run -- execute        # signatory B: discover on-chain + fetch + verify + execute
```

Success = `execute` discovers the pending op (no hash given), prints the integrity-OK
line, and reports `inner call dispatched OK` (the recipient's balance increases) — with
B having pulled the call data only from the Bulletin Chain.

### Queue several, approve one by one (accountant → management)

Person A (accountant) queues multiple **distinct** transactions; person B (management)
reviews and approves them individually:

```bash
cargo run -- init --amount 1000000000                 # queue: 0.1 PAS → B
cargo run -- init --amount 2000000000                 # queue: 0.2 PAS → B (distinct hash)
cargo run -- init --recipient 5Fxx… --amount 5000000000   # queue: 0.5 PAS → someone else
cargo run -- pending                                  # B reviews the queue (calls decoded from Bulletin)
cargo run -- execute --call-hash 0x<one>              # B approves one…
cargo run -- execute                                  # …or processes all remaining
```

Each transaction must be distinct — an identical call produces the same hash, and the
pallet rejects a second first-approval with `Multisig::NoTimepoint`. `init` guards
against that and tells you.

**Identical payments (same payee + amount, different invoices):** add `--memo <ref>`.
It wraps the transfer + a `system.remark_with_event(<ref>)` in a `utility.batch_all`,
so the call hash differs per invoice while the transfer is identical — and the memo is
an on-chain audit reference that `pending` shows decoded:

```bash
cargo run -- init --memo invoice-001    # 0.1 PAS → B, audit ref invoice-001
cargo run -- init --memo invoice-002    # SAME payment, distinct hash
cargo run -- pending                     # shows  invoice : "invoice-001" / "invoice-002"
```

## Endpoints

- Paseo Asset Hub RPC: `wss://asset-hub-paseo-rpc.n.dwellir.com`
- Paseo Bulletin RPC (Next v2): `wss://paseo-bulletin-next-rpc.polkadot.io`
- Bulletin IPFS gateway: `https://paseo-bulletin-next-ipfs.polkadot.io/ipfs/<CID>?format=raw`

Metadata is pinned under `metadata/*.scale`. Asset Hub needs **full** metadata (no
`--pallets`) — slimming prunes transaction-extension types (`ChargeAssetTxPayment`,
`CheckMetadataHash`, …) and breaks signing. Regenerate with:
```bash
subxt metadata --url wss://asset-hub-paseo-rpc.n.dwellir.com -f bytes -o metadata/assethub.scale
subxt metadata --url wss://paseo-bulletin-next-rpc.polkadot.io --pallets TransactionStorage,System -f bytes -o metadata/bulletin.scale
```

## Scope / known simplifications

- 2-of-2 threshold.
- `execute` auto-discovers all pending ops by iterating `Multisig.Multisigs`; pass
  `--call-hash 0x…` to target a specific one.
- `max_weight` for `as_multi` is a generous constant (refunded down to actual).
- Retrieval uses the hosted IPFS gateway for convenience; peer-to-peer / own-node fetch
  (removing the last central touchpoint) is future hardening — see *Trust model*.
- Long-pending ops aren't renewed, so they're bound by Bulletin's ~14-day retention.
- Throwaway POC — not the production narada app.
