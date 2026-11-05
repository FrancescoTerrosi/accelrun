# Publishing checklist — pre-push / push / post-push

Maintainer operations guide for publishing and releasing accelrun.
Everything here is safe to be public: it documents *how* the project is
published, which is part of its reproducibility story.

---

## 1. Pre-push — verify the local repo

```bash
# clean tree, single root commit on main, correct author
git status --porcelain                    # must print nothing
git log --oneline                         # one commit: "Initial public import"
git log -1 --pretty='%an <%ae>'
#   -> Francesco Terrosi <33395865+FrancescoTerrosi@users.noreply.github.com>

# no placeholders or noise anywhere (searches ALL history, not just the tree)
git grep -n "<owner>\|2025" $(git rev-list --all)   # must print nothing

# the ignore set is intact — these must stay out of the public repo
git status --porcelain --ignored | grep '^!!'
#   -> .cargo/  accelrun  archive/  target/

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
      releasing / TODO)
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
Danger Zone → **Change repository visibility → Make public**. Why this
order and not straight-to-public, and not stay-private-longer: private→
public loses nothing (commit dates carry provenance and were set at commit
time, not push time), but Zenodo only syncs **public** repositories and
the public timestamp is itself provenance protection — an inspection
afternoon, not a holding pattern.

## 3. Post-push — release machinery

### 3a. What Zenodo is, and why this repo uses it

[Zenodo](https://zenodo.org) is an open repository for research outputs,
operated by CERN on behalf of the European Commission (OpenAIRE). For this
project the relevant feature is the **GitHub integration**: every tag pushed
to GitHub is automatically archived as a versioned snapshot with a
**permanent DOI**, free of charge. That turns a release into a citable
scholarly object — papers can cite `accelrun v0.1.0` by DOI, and the DOI's
landing page always points back at the GitHub release. One project gets two
identifiers: a *concept DOI* (cites "accelrun" across all versions, use this
in papers) and one DOI per version. GitHub itself hosts code but issues no
DOIs; Zenodo is the missing citation layer.

Setup (one time):

1. Sign in at zenodo.org (ORCID or GitHub login both work)
2. Settings → Applications → GitHub → "Sync now"
3. Flip the switch next to `accelrun` to **ON** (the repository must be
   public by this point — Zenodo only lists public repositories)

### 3b. Tag the first release

```bash
# annotated tag (add -s instead of -a for a GPG-signed tag once a key exists)
git tag -a v0.1.0 -m "accelrun 0.1.0"
git push origin v0.1.0
```

Pushing the tag makes Zenodo archive the snapshot and mint the DOIs.
Record the concept DOI in the README citation section afterwards.

### 3c. GitHub Release with verified binaries

```bash
# build the release binary (no Rust needed)
docker build -t accelrun:latest .
docker create --name ar accelrun:latest
docker cp ar:/usr/local/bin/accelrun ./accelrun-v0.1.0-x86_64
docker rm ar
sha256sum accelrun-v0.1.0-x86_64 > SHA256SUMS
```

GitHub → Releases → *Draft a new release* → choose tag `v0.1.0` → add release
notes → attach **both** `accelrun-v0.1.0-x86_64` and `SHA256SUMS`.

### 3d. New-user simulation (the last gate)

```bash
git clone https://github.com/FrancescoTerrosi/accelrun.git /tmp/ar-test
cd /tmp/ar-test
docker build -t accelrun:latest .
docker create --name ar accelrun:latest \
  && docker cp ar:/usr/local/bin/accelrun . && docker rm ar
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
- **Releases**: tag → Zenodo DOI → GitHub release with binaries + checksums;
  keep `CITATION.cff` `date-released` in sync with tag dates.
- **Contributions**: when the first outside PR arrives, add a short
  contributor note (inbound = outbound, MIT OR Apache-2.0) BEFORE merging —
  sole copyright is what preserves the dual-licensing/relicensing freedom.
- **Roadmap drives majors**: podman support, k8s integration via the CDI
  surface, MIG-aware capping, numeric-drift testing.
