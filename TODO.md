# TODO — project state and next steps

Last updated: 2026-11-05 (promotion session). This file is the single-glance
state of the whole accelrun effort: what landed, what is next, and where the
detail lives (`docs/releasing.md` for publishing mechanics, `README.md` for
the public roadmap).

## ✅ Done — promotion to standalone repository (2026-11)

- [x] Public-grade README — quickstart in the first lines, honest scope, verified status
- [x] Dual license **MIT OR Apache-2.0** — license files, `Cargo.toml` SPDX, README, CITATION.cff
- [x] Copyright: Francesco Terrosi, 2026; public commit identity via GitHub noreply address
- [x] Repo hygiene — `.gitignore` (build artifacts, host-specific linker config, local archive)
- [x] Clean history born without dev noise — `f47ccb8` "Initial public import", `3cf72e0` docs
- [x] `docs/decisions.md` distilled — foundational decisions, five-bug chain, 2026 promotion entry; full original in local `archive/` (gitignored)
- [x] AI-assistance disclosure — README section + decisions-log preamble
- [x] Citation metadata — `CITATION.cff` (author, v0.1.0, date-released, keywords)
- [x] Provenance strategy — signed releases + Zenodo DOIs planned; sole-copyright policy noted
- [x] Publishing checklist — `docs/releasing.md` (pre-push / push / post-push)

## 🔲 Now — go live (mechanics: docs/releasing.md §1–3)

- [ ] Create the GitHub repo `FrancescoTerrosi/accelrun` as **private** (**no** auto-init)
- [ ] `git remote add origin https://github.com/FrancescoTerrosi/accelrun.git && git push -u origin main`
- [ ] Inspect while private: README rendering, doc links, "Cite" button, noreply author (releasing.md §2)
- [ ] Web UI: description + topics (`gpu`, `docker`, `containers`, `cdi`, `nvidia`, `rust`)
- [ ] Flip to **public** (Settings → Danger Zone → Make public) — Zenodo only syncs public repos
- [ ] Zenodo: enable the GitHub integration for this repo (DOIs per release)
- [ ] Tag `v0.1.0` → push → Zenodo mints concept + version DOIs
- [ ] GitHub Release: notes + `accelrun-v0.1.0-x86_64` + `SHA256SUMS`
- [ ] Add the Zenodo **concept DOI** to README's citation section once minted
- [ ] New-user simulation: fresh clone → build → `probe` → smoke test PASS (the release gate)

## 🔲 Short term — first weeks after publishing

- [ ] CI: GitHub Actions — `cargo build` + `cargo test` (zero-dep makes this trivial)
- [ ] Signed commits/tags (GPG key; `git config commit.gpgsign true`)
- [ ] Optional: `cargo publish` to crates.io (metadata already complete)
- [ ] Contributor note (**inbound = outbound, MIT OR Apache-2.0**) *before* merging the first outside PR — sole copyright is what preserves the relicensing freedom
- [ ] Triage incoming issues from the runbook failure map; enrich docs from recurring questions

## 🔲 Medium term — publication path

- [ ] arXiv preprint (cs.DC): vendor-neutral accelerator resource abstraction + unified memory as first-class + fault model; accelrun as reference implementation
- [ ] JOSS tool paper — the repo already meets the bar (OSI license, quickstart, smoke test); submit after v0.1.0 + DOI exist
- [ ] End-to-end verification of a **second backend (AMD ROCm)** — required to substantiate the portability thesis
- [ ] Quantitative evaluation for the systems paper: attach overhead vs nvidia-container-toolkit (startup latency, footprint), cap accuracy on unified backends, workload-level parity
- [ ] Fault-injection study of the FMEA registry (each fault: inject → detect → mitigate) — the most novel research angle
- [ ] Numeric-drift study across backends for one IR-compiled workload

## 🔲 Long term — engineering roadmap

- [ ] podman support (launcher currently hardcodes docker)
- [ ] K8s integration via the CDI spec surface
- [ ] MIG-aware NVIDIA hard memory caps
- [ ] Unified-memory cgroup policies (the standards/position-paper hook)

---

Maintenance: move finished items into the Done section; keep each item a
pointer to where the detail lives.
