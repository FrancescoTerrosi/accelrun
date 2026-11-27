# Publishing checklist — pre-push / push / post-push

Maintainer operations guide for publishing and releasing accelrun.

> **v0.1.0 has shipped.** §2 (private→public flip) and §3a (Zenodo setup) are
> one-time steps, already done. For a repeat release (e.g. v0.1.1): §1 → §3b →
> §3c → §3d.

---

## 1. Pre-push — verify the local repo

```bash
# clean tree, correct author
git status --porcelain                    # must print nothing
git log --oneline -5                      # recent commits
git log -1 --pretty='%an <%ae>'
#   -> Francesco Terrosi <33395865+FrancescoTerrosi@users.noreply.github.com>

# no placeholders anywhere (searches ALL history, not just the tree)
# (this doc is excluded: history carries the old pattern literal, which would self-match)
git grep -n "<owner>" $(git rev-list --all) -- ':!docs/releasing.md'   # must print nothing

# the ignore set is intact — these must stay out of the public repo
git status --porcelain --ignored | grep '^!!'
#   -> .cargo/  accelrun  accelrun-v*  target/
#   (SHA256SUMS is NOT ignored — it is committed as the trusted checksum anchor)

# licenses + citation metadata
grep Copyright LICENSE-MIT                # (c) 2026 Francesco Terrosi
grep -n "version\|date-released" CITATION.cff
```

Checklist before touching GitHub:

- [ ] `README.md` quickstart clone URL is the real repo URL
- [ ] `CITATION.cff` `date-released` matches the planned tag date
- [ ] `docs/decisions.md` has no personal/host/project-origin noise
- [ ] Both LICENSE files present and referenced from README, Cargo.toml, CITATION.cff

## 2. Push — private for inspection, then flip public

On github.com (web UI): **New repository** → owner `FrancescoTerrosi`,
name `accelrun`, **Private** — leave every initialization option OFF
(no README, no .gitignore, no license: the push carries all of them and
GitHub-side initialization would conflict).

```bash
git remote add origin https://github.com/FrancescoTerrosi/accelrun.git
git push -u origin main
```

Inspect while private — the GitHub-side render gate:

- [ ] README lands as intended (quickstart copy-paste, tables, code blocks)
- [ ] every Documentation link resolves (runbook / vision / decisions /
      releasing)
- [ ] "Cite this repository" opens without a CITATION.cff validation warning
- [ ] history shows the noreply author with the right avatar
- [ ] description + topics set (they carry into public)

Set while private (carries over on the flip):

- Description: `Toolkit-free GPU attach for containers — one binary, any
  Docker version, no NVIDIA Container Toolkit.`
- Topics: `gpu`, `docker`, `containers`, `cdi`, `nvidia`, `rust`
- Optional: branch protection on `main` (require PR reviews once
  contributors appear; pointless while solo)

Fix anything found, commit, push. Then flip: Settings → General →
Danger Zone → **Change repository visibility → Make public**. Private→
public loses nothing, but Zenodo only syncs **public** repositories, and the
public timestamp is provenance protection.

## 3. Post-push — release machinery

### 3a. What Zenodo is, and why this repo uses it

[Zenodo](https://zenodo.org) is an open research-output repository. Its
**GitHub integration** archives every tagged release as a versioned
snapshot with a **permanent DOI**, free of charge: a *concept DOI* cites
"accelrun" across all versions, and one DOI per version points back at the
GitHub release.

Setup (one time):

1. Sign in at zenodo.org (ORCID or GitHub login both work)
2. Settings → Applications → GitHub → "Sync now"
3. Flip the switch next to `accelrun` to **ON** (the repository must be
   public by this point — Zenodo only lists public repositories)

One Zenodo quirk before the first tag: since Zenodo's InvenioRDM
migration (late 2023), its metadata importer requires `license` in
`CITATION.cff` to be a **single SPDX string** and crashes on the
spec-valid array form (`license: [MIT, Apache-2.0]`) with
`Citation metadata load failed` (zenodo/zenodo#2515, still open as of
2026-11). The dual license is unaffected: the grant lives in
`LICENSE-MIT`/`LICENSE-APACHE` and `Cargo.toml`'s SPDX expression, so the
CFF carries `license: MIT` and the Zenodo record description must state
"Dual-licensed under MIT OR Apache-2.0; both license texts are in the
archive." Watch #2515 and restore the array if it is ever fixed.

### 3b. Tag the release

```bash
# annotated tag (add -s instead of -a for a GPG-signed tag once a key exists)
git tag -a v0.1.1 -m "accelrun 0.1.1"
git push origin v0.1.1
```

Publishing the release for this tag is the trigger: Zenodo archives the
snapshot and mints the DOIs on the release event, not on bare tag pushes.
Record the concept DOI in the README citation section afterwards.

### 3c. GitHub Release with verified binaries

```bash
# build + verify the release binary and its checksum (no Rust needed)
./build.sh --release v0.1.1
#   -> accelrun-v0.1.1-x86_64   (verified ELF + ACCEL_* signature + probe smoke)
#   -> SHA256SUMS               (checksum appended; commit it with the release)

# the checksum record is committed, so anchor it before tagging:
git add SHA256SUMS && git commit -m "release: v0.1.1 checksums"
```

> The committed `SHA256SUMS` record **begins at v0.1.1**. v0.1.0 shipped
> before checksum-anchoring existed, so it has no committed checksum (and its
> release binary was published as an unversioned `accelrun` asset, so there is
> nothing clean to backfill). Future releases append one line each.

GitHub → Releases → *Draft a new release* → choose tag `v0.1.1` → add release
notes → attach the `accelrun-v0.1.1-x86_64` binary (and a copy of
`SHA256SUMS` for convenience). The committed `SHA256SUMS` — not the
release-page copy — is the trust anchor third parties verify against.

Publish **only after** the assets are attached: Zenodo snapshots the
release at publish time — binaries added to an already-published release
do not reach the archived record (the only re-trigger is delete +
re-publish the release).

### 3d. New-user simulation (the last gate)

```bash
git clone https://github.com/FrancescoTerrosi/accelrun.git /tmp/ar-test
cd /tmp/ar-test
./build.sh                              # build + verify ./accelrun
./accelrun probe                       # backends + diagnostics listed
# then the README Quickstart commands, ending with tests/gpu-smoke.py on the
# GPU host — PASS is the release gate
```

### 3e. Optional: crates.io

`Cargo.toml` already carries `license`, `readme`, `keywords`, `categories`,
so `cargo login <token>` + `cargo publish` works as-is when the time comes.

## 4. Ongoing policy

- **Commits**: conventional (`feat:`, `fix:`, `docs:`, `chore:`) — organic
  history starts at the second commit; the promotion is the first.
- **Decision log**: append new entries to `docs/decisions.md` as decisions
  happen; it is the project's technical memory.
- **Releases**: single-`license` CFF check → tag → GitHub release with binaries + checksums attached **before** publish (Zenodo snapshots at publish) → DOI; keep `CITATION.cff` `date-released` in sync with tag dates.
- **Contributions**: when the first outside PR arrives, add a short
  contributor note (inbound = outbound, MIT OR Apache-2.0) BEFORE merging —
  sole copyright is what preserves the dual-licensing/relicensing freedom.
- **Roadmap drives majors**: podman support, k8s integration via the CDI
  surface, MIG-aware capping, numeric-drift testing.
