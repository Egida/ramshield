# RamShield production release process

Production release means a verified artifact, not a green compile. The release unit is:

`source commit + version/tag + Cargo.lock + config profile + metric-keystore hash + artifact digest + capability/runtime state + rollback reference`

## 1. Release candidate on `p1`

1. Freeze scope. One feature per commit; no unrelated working-tree changes.
2. Record:
   - branch and commit;
   - `Cargo.toml` version;
   - latest tag;
   - config profile and secret source;
   - metric-keystore and generated JSONL hashes;
   - target host/kernel/XDP capability state;
   - current daemon PID/listeners.
3. Keep open findings visible. F5 (partial subnet coverage) and F1 (production SHM `.expect()`) block a claim of production-ready security, even if the release candidate builds.
4. Run the narrow feature check RED-first, then the complete review gate:

```bash
scripts/review_pipeline.sh
```

5. Build the exact release binary:

```bash
cargo build --release --locked --features full
sha256sum target/release/ramshield Cargo.lock
```

`cargo build --release` replaces the ELF and removes file capabilities. Re-apply and verify `cap_net_admin,cap_perfmon,cap_bpf=eip` before any XDP test.

6. Run production-like smoke against isolated loopback ports and scratch WAL state:

```bash
CFG=config.prod.toml.example \
IPC_PORT=17890 DASH_ADDR=127.0.0.1:19999 \
WAL_DIR=/tmp/ramshield-release-wal \
bash scripts/prod_smoke.sh
```

The smoke test must own its daemon PID. It must never kill an unrelated production process.

7. Run the canonical live suite against its own scratch server:

```bash
python3 scripts/final_integration.py
```

8. Audit the runtime log. Unexplained panic, high-severity finding, parse rejection, unknown reason, or failed fixture blocks release.

## 2. Artifact gate

The Containerfile is the production artifact definition. Before publishing an image:

```bash
docker build --pull --no-cache \
  --label org.opencontainers.image.revision="$(git rev-parse HEAD)" \
  --label org.opencontainers.image.version="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')" \
  -t ghcr.io/grep999/ramshield:<version>-rc<N> .
```

Record the immutable image digest. Never deploy a mutable tag as identity. Verify the image contains the expected binary and starts with an explicit `--config` path.

If Docker or registry access is unavailable, mark the artifact boundary unverified. Do not call the release production-ready.

## 3. Promotion to `master`

1. Fetch all remotes. Inspect remote `master`; never overwrite it.
2. Integrate remote `master` into `p1` first. Resolve README or other conflicts deliberately.
3. Re-run the full gate on the integrated `p1`.
4. Push `p1` only if the remote branch is an ancestor of local `p1`.
5. Fast-forward local `master` to the verified `p1` tip. Refuse non-fast-forward promotion.
6. Push `master` explicitly. Push the annotated release-candidate tag only after post-merge tests pass.

Required promotion checks:

```bash
git merge-base --is-ancestor grep999/master p1
git merge-base --is-ancestor p1 master
```

The second check is evaluated after local `master` is fast-forwarded. No force-push. No reset-hard. No amend.

## 4. Post-push test on `master`

After pushing `master`, fetch the remote ref and verify it resolves to the intended commit. Test that exact checkout/commit:

```bash
git fetch grep999 master --tags
git rev-parse grep999/master
scripts/review_pipeline.sh
cargo test --workspace --locked --features full
CFG=config.prod.toml.example \
IPC_PORT=17890 DASH_ADDR=127.0.0.1:19999 \
WAL_DIR=/tmp/ramshield-master-smoke-wal \
bash scripts/prod_smoke.sh
```

The result must distinguish:

- source/build/test pass;
- production-like no-XDP pass;
- privileged XDP boundary pass or unverified;
- OCI image build/digest pass or unverified;
- known open findings.

## 5. Rollback

Rollback target is the previous verified commit/image pair, not an ad-hoc rebuild:

```bash
git revert <release-commit>       # normal shared-branch rollback
docker pull ghcr.io/grep999/ramshield@sha256:<known-good-digest>
```

Restore the previous config and WAL policy. Verify health, listeners, auth, snapshot, stream, metrics, and block/unblock behavior before reopening traffic.

## Release claim

A release candidate may be promoted to `master` while production blockers remain documented. It may not be labeled production-ready until F1/F5 are closed, an immutable OCI digest is recorded, WAL restart recovery is verified, and privileged XDP smoke passes on the target deployment class.
