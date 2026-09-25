# Delivery checklist — one engagement

Copy into the pack directory as `CHECKLIST.md` and tick.

## Before work

- [ ] Scope one-pager signed (product, version, NIP/KRS, signatory)
- [ ] Engagement letter signed (boundaries verbatim)
- [ ] 50% invoiced
- [ ] `python3 assess.py --selfcheck` PASS
- [ ] NDA / DPA position if SBOM or recordings will be seen

## Run

- [ ] `answers.json` complete (`yes|partial|no|na`; weight-3 `na` has `_na_why`)
- [ ] `python3 pack.py --answers answers.json --evidence evidence/ --out pack/ ...`
- [ ] `report.md` verdict is not READY unless weight-3 gaps are actually closed
- [ ] `manifest.json` has answers SHA-256 and a hash per evidence file
- [ ] Optional: ephemeral Ed25519 over manifest via `q4_pocs/release-attest` — destroy the key after packing. Never reuse `bot-gate/attest/attest_key.pem`.

## Hand-off

- [ ] Blocking gaps named with the evidence that would close them
- [ ] CSIRT named (NASK default private; GOV public admin; MON military)
- [ ] If Class II / critical / Class I without harmonised standards → NANDO body named
- [ ] If they asked for ISO → PCA/IAF body named, no commission
- [ ] If they asked for a legal opinion → a lawyer named, stop
- [ ] 50% invoiced on delivery
- [ ] Engagement closed. Next line (#2 / #3 / #4 / #5) is a new signature.

## Do not

- File S46 or UODO as the client
- Sell the KEV∩SBOM engine until it exists
- Leave DEMO CVE data unmarked
