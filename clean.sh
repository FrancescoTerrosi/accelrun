#!/usr/bin/env bash
# clean.sh — remove every artifact accelrun may leave, inside and outside the repo.
#
# Same philosophy as the tool itself: --dry-run first, match by evidence
# (the ACCEL_* env contract), never touch global docker state.
#
# Usage: ./clean.sh [--dry-run] [-y|--yes] [--all] [--repo-only] [-h|--help]

usage() {
  cat <<'EOF'
clean.sh — remove accelrun artifacts, inside and outside the repo

Inventory (what the project leaves behind, and where):

  repo        target/, ./accelrun, ./accelrun-v* (local convenience/release
              binaries), tests/__pycache__/                     [in this clone]
  containers  every container accelrun launched — matched by the ACCEL_*
              env contract or the accelrun image, NOT by name guessing;
              catches `-d`/`--restart` leftovers (docker drops --rm for
              those) and the `ar` helper container from the quickstart
  networks    `accel` (README recipe) + user-created networks of accelrun
              containers; docker-compose-owned networks are skipped
  volumes     `ollama-data` (README recipe — LLM models, re-download needed!)
              + named volumes mounted by accelrun containers;
              anonymous volumes are only reported (never auto-pruned)
  images      accelrun:* (+ base images rust:*/debian:bookworm-slim via --all)
  filesystem  /tmp/accelrun-cdi (CDI spec written on EVERY `run`),
              CDI specs registering accel.local/gpu in /etc/cdi or
              /var/run/cdi (--cdi mode), /sys/fs/cgroup/accelrun
              (cgroup v2 memory cap on unified backends)
  via --all   ~/.config/accelrun/ (custom probes.toml — user content),
              accelrun binaries found on $PATH (verified by ELF magic +
              ACCEL_* string signature), /tmp/fakelib (dev-host linker
              workaround — cargo build breaks until restored, see
              .cargo/config.toml), /tmp/ar-test (release-verification clone)

Flags:

  --dry-run    print what would be cleaned, change nothing
  -y, --yes    assume yes for everything in scope
  --all        extend scope to user-owned items (config, installed binaries,
               dev-host /tmp dirs, base images)
  --repo-only  only in-repo build artifacts (target/, ./accelrun, ...)
  -h, --help   this text

Safety model:

  - never runs docker system/builder/volume prune — nothing global
  - containers matched via their ACCEL_* env contract (injected by
    accelrun in both direct and cdi modes) or built from the accelrun image
  - relies on docker's own in-use checks (volume rm / network rm fail
    cleanly while foreign containers still reference them)
  - interactive prompts by default; in non-interactive shells without -y,
    every prompt auto-answers "no"
  - needs root only for: CDI specs under /etc/cdi|/var/run/cdi, the
    /sys/fs/cgroup/accelrun cgroup, binaries outside $HOME

Follow-up (deliberately left to you, manual):
  docker build cache keeps the ~1 GB rust builder layers:
    docker system df -v          # inspect
    docker builder prune         # if and when you decide to
EOF
}

set -u
shopt -s nullglob

DRY_RUN=false ASSUME_YES=false ALL=false REPO_ONLY=false
for arg in "$@"; do
  case "$arg" in
    --dry-run)   DRY_RUN=true ;;
    -y|--yes)    ASSUME_YES=true ;;
    --all)       ALL=true ;;
    --repo-only) REPO_ONLY=true ;;
    -h|--help)   usage; exit 0 ;;
    *) echo "unknown arg: $arg (try --help)" >&2; exit 2 ;;
  esac
done

repo_dir="$(cd "$(dirname "$0")" && pwd)"

echo "accelrun cleanup — repo: $repo_dir"
$DRY_RUN    && echo "(dry-run: nothing will be changed)"
$ALL        && echo "(--all: user-owned artifacts included)"
$REPO_ONLY  && echo "(--repo-only: in-repo build artifacts only)"

# unattended real deletions get one short escape hatch (today's lesson: with
# -y there are no prompts; give operators 3 s to abort)
if $ASSUME_YES && ! $DRY_RUN; then
  echo "!! -y without --dry-run: everything in scope is removed WITHOUT prompts."
  echo "!! Press Ctrl-C within 3 s to abort."
  sleep 3
fi

# ---------------------------------------------------------------- helpers ---

removed=0

# ask_yes DESC — 0 = proceed, 1 = keep. Dry-run and -y short-circuit to yes;
# a non-interactive stdin auto-answers "no".
ask_yes() {
  $DRY_RUN     && return 0
  $ASSUME_YES  && return 0
  [ -t 0 ]     || return 1
  local r
  read -r -p "  $1 [y/N] " r < /dev/tty 2>/dev/null || return 1
  [[ $r =~ ^[Yy]([Ee][Ss])?$ ]]
}

# act CMD... — run unless dry-run; on success count the removal.
# Failures are reported inline, never fatal (docker refuses unsafe rm's).
act() {
  if $DRY_RUN; then echo "      would run: $*"; return 0; fi
  if "$@" 2>&1; then
    removed=$((removed + 1))
  else
    echo "      (failed — kept in place)"
  fi
}

# try_item DESC CMD... — ask, then act.
try_item() {
  local desc="$1"; shift
  if $DRY_RUN; then echo "      would run: $*"; return 0; fi
  if ask_yes "$desc"; then
    act "$@"
  else
    echo "      kept: $desc"
  fi
}

docker_ok() { docker version >/dev/null 2>&1; }

# uniq_append X... — appends elements to the global `candidates` array,
# skipping ones already present (hardcoded recipe names overlap collected ones).
uniq_append() {
  local x
  for x in "$@"; do
    [ -n "$x" ] || continue
    [[ ! " ${candidates[*]:-} " == *" $x "* ]] && candidates+=("$x")
  done
}

# -------------------------------------------------------------------- repo ---

echo
echo "── repo artifacts (in $repo_dir) ──"

# local convenience/release binaries (gitignored); name-based, never matches
# clean.sh itself — exact name + release-suffix globs only.
local_bins=()
[ -f "$repo_dir/accelrun" ]   && local_bins+=("$repo_dir/accelrun")
local_bins+=("$repo_dir"/accelrun-v*)
if [ ${#local_bins[@]} -gt 0 ]; then
  try_item "remove local binary(es): ${local_bins[*]}" rm -f -- "${local_bins[@]}"
else
  echo "  no local binaries — ok"
fi

if [ -d "$repo_dir/target" ]; then
  try_item "remove cargo build dir target/" rm -rf -- "$repo_dir/target"
  [ -d "$repo_dir/target" ] && echo "  target/ still present" || echo "  target/ gone"
else
  echo "  target/ — ok"
fi

if [ -d "$repo_dir/tests/__pycache__" ]; then
  try_item "remove tests/__pycache__/" rm -rf -- "$repo_dir/tests/__pycache__"
else
  echo "  tests/__pycache__/ — ok"
fi

# ---------------------------------------------------------------- docker ---

if $REPO_ONLY; then
  echo
  echo "(--repo-only: skipping docker and host-filesystem sections)"
else
  if ! docker_ok; then
    echo
    echo "── docker artifacts ──"
    echo "  docker not available/running — skipping docker sections"
  else
    containers=()        # "id|name|status"
    vol_names=()         # named volumes mounted by accelrun containers
    net_names=()         # user-created networks of accelrun containers
    anon_vol_seen=false

    echo
    echo "── docker containers launched by accelrun ──"
    while IFS='|' read -r cid cname cstat; do
      [ -n "$cid" ] || continue
      envs="$(docker inspect "$cid" --format '{{range .Config.Env}}{{.}};{{end}}' 2>/dev/null)" || continue
      img="$(docker inspect "$cid" --format '{{.Config.Image}}' 2>/dev/null)"
      # Evidence, not assumptions: the ACCEL_* contract reaches Container Env
      # in BOTH modes (direct: -e; cdi: containerEdits.env applied by docker)
      # and anything built from the accelrun image is ours (quickstart `ar`).
      if [[ $envs == *ACCEL_BACKEND=* || $envs == *ACCEL_MEM_BYTES=* || $img == accelrun:* ]]; then
        containers+=("$cid|$cname|$cstat")
        # collect mounted named volumes + non-default networks for later steps
        for v in $(docker inspect "$cid" --format \
            '{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}' 2>/dev/null); do
          if [[ $v =~ ^[0-9a-f]{64}$ ]]; then
            anon_vol_seen=true
          elif [[ ! " ${vol_names[*]:-} " == *" $v "* ]]; then
            vol_names+=("$v")
          fi
        done
        for n in $(docker inspect "$cid" --format \
            '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}' 2>/dev/null); do
          case $n in bridge|host|none) continue ;; esac
          # leave docker-compose-owned networks alone (foreign project state)
          comp_net="$(docker network inspect "$n" \
            --format '{{if index .Labels "com.docker.compose.network"}}compose{{else}}plain{{end}}' 2>/dev/null)"
          if [ "$comp_net" != "compose" ] \
             && [[ ! " ${net_names[*]:-} " == *" $n "* ]]; then
            net_names+=("$n")
          fi
        done
      fi
    done < <(docker ps -a --format '{{.ID}}|{{.Names}}|{{.Status}}')

    if [ ${#containers[@]} -eq 0 ]; then
      echo "  none — ok"
    else
      for c in "${containers[@]}"; do
        IFS='|' read -r cid cname cstat <<<"$c"
        try_item "remove container $cname (status: $cstat; running ones are stopped)" \
          docker rm -f -- "$cname"
      done
    fi
    if ! $DRY_RUN && $anon_vol_seen && [ ${#vol_names[@]} -eq 0 ]; then
      echo "  note: anonymous volumes were mounted by those containers —"
      echo "        list leftovers: docker volume ls -f dangling=true"
      echo "        (never auto-pruned here)"
    fi

    echo
    echo "── docker networks ──"
    net_found=false
    candidates=()
    uniq_append accel "${net_names[@]:-}"
    for n in "${candidates[@]}"; do
      [ -n "$n" ] || continue
      docker network inspect "$n" >/dev/null 2>&1 || continue
      net_found=true
      try_item "remove docker network \"$n\"" docker network rm "$n"
    done
    $net_found || echo "  none — ok"

    echo
    echo "── docker volumes (accelrun recipes: LLM models live here) ──"
    vol_found=false
    candidates=()
    uniq_append ollama-data "${vol_names[@]:-}"
    for v in "${candidates[@]}"; do
      [ -n "$v" ] || continue
      docker volume inspect "$v" >/dev/null 2>&1 || continue
      vol_found=true
      try_item "remove docker volume \"$v\" — models inside are gone and must be re-downloaded" \
        docker volume rm "$v"
    done
    $vol_found || echo "  none — ok"

    echo
    echo "── docker images ──"
    imgs=()
    while IFS= read -r img; do
      [[ "$img" != *":<none>"* ]] && imgs+=("$img")
    done < <(docker images accelrun --format '{{.Repository}}:{{.Tag}}')
    if [ ${#imgs[@]} -eq 0 ]; then
      echo "  no accelrun images — ok"
    else
      for img in "${imgs[@]}"; do
        try_item "remove docker image $img" docker rmi "$img"
      done
    fi

    if $ALL; then
      echo
      echo "── docker base images (--all, shared with other projects) ──"
      for img in rust:1.99-bookworm debian:bookworm-slim; do
        docker image inspect "$img" >/dev/null 2>&1 || continue
        deps="$(docker ps -a --filter ancestor="$img" --format '{{.Names}}' | head -3 | tr '\n' ' ')"
        if [ -n "$deps" ]; then
          echo "  keep $img — still referenced by: $deps"
        else
          try_item "remove base image $img" docker rmi "$img"
        fi
      done
    fi

    echo
    echo "  hint: build cache (~1 GB rust builder layers) may remain:"
    echo "        docker system df -v   ← inspect, then prune manually if desired"
  fi
fi

# ---------------------------------------------------------- host filesystem ---

if $REPO_ONLY; then
  :
else
  is_root=false
  [ "$(id -u)" -eq 0 ] && is_root=true

  echo
  echo "── host filesystem: CDI artifacts ──"
  # /tmp/accelrun-cdi: the spec written by EVERY `accelrun run` (even direct
  # mode) — always safe to remove, recreated on next run.
  if [ -d /tmp/accelrun-cdi ]; then
    try_item "remove /tmp/accelrun-cdi (CDI spec dir)" rm -rf -- /tmp/accelrun-cdi
  else
    echo "  /tmp/accelrun-cdi — ok"
  fi
  # CDI specs installed into the daemon dir for `--cdi` mode: registration is
  # proven by CONTENT (kind accel.local/gpu), file names may vary.
  cdi_found=false
  for cdi_dir in /var/run/cdi /etc/cdi; do
    [ -d "$cdi_dir" ] || continue
    for f in "$cdi_dir"/*; do
      [ -f "$f" ] || continue
      [ -r "$f" ] || { echo "  $f exists but is not readable (root needed)"; continue; }
      if grep -qs 'accel.local/gpu' "$f"; then
        cdi_found=true
        try_item "remove installed CDI spec $f — daemon will deregister accel.local/gpu on reload" \
          rm -f -- "$f"
      fi
    done
  done
  $cdi_found || echo "  no accelrun CDI specs installed — ok"
  if ! $DRY_RUN && $cdi_found && ! $is_root; then
    echo "  note: if a removal above failed you may need: sudo $0"
  fi

  echo
  echo "── host filesystem: cgroup v2 memory cap ──"
  # /sys/fs/cgroup/accelrun: created on unified-backend runs (memory.max =
  # budget hint; cgroup.procs points at the accelrun process itself, which is
  # gone after exit). Removal needs root and an empty dir (the kernel refuses
  # rmdir while members/children remain).
  if [ -d /sys/fs/cgroup/accelrun ]; then
    if $DRY_RUN; then
      echo "      would run: rmdir /sys/fs/cgroup/accelrun (recursively, deepest first)"
    elif ask_yes "remove /sys/fs/cgroup/accelrun ($(ls /sys/fs/cgroup/accelrun | wc -l) entries)"; then
      find /sys/fs/cgroup/accelrun -mindepth 1 -type d -print0 2>/dev/null | \
        while IFS= read -r -d '' d; do rmdir "$d" 2>/dev/null; done
      if rmdir /sys/fs/cgroup/accelrun 2>/dev/null; then
        removed=$((removed + 1))
        echo "      · removed"
      else
        echo "      · (failed — needs root, or accelrun processes are still inside)"
      fi
    else
      echo "      kept."
    fi
  else
    echo "  /sys/fs/cgroup/accelrun — ok"
  fi

  if $ALL; then
    echo
    echo "── host filesystem: user-owned artifacts (--all) ──"
    # custom probes config — real user content, hence --all-gated
    if [ -e "$HOME/.config/accelrun" ]; then
      try_item "remove $HOME/.config/accelrun — your probes.toml lives here; re-create from specs/probes.toml.example if needed" \
        rm -rf -- "$HOME/.config/accelrun"
    else
      echo "  $HOME/.config/accelrun — ok"
    fi
    # installed binaries on $PATH (this repo's ./accelrun is excluded above);
    # offered only after verifying the ELF + accelrun string signature
    prev_ifs="$IFS"; IFS=':'
    path_dirs=($PATH "$HOME/.local/bin")
    IFS="$prev_ifs"
    inst_bins=()
    for d in ${path_dirs[@]}; do
      [ -n "$d" ] || continue
      p="$d/accelrun"; [ -x "$p" ] || continue
      [ "$(cd "$(dirname "$p")" && pwd)" = "$repo_dir" ] && continue
      head -c 4 "$p" 2>/dev/null | grep -aq ELF       || continue
      grep -aq 'ACCEL_' "$p" 2>/dev/null              || continue   # ours indeed
      inst_bins+=("$p")
    done
    if [ ${#inst_bins[@]} -eq 0 ]; then
      echo "  no installed accelrun binaries outside the repo — ok"
    else
      for p in "${inst_bins[@]}"; do
        try_item "remove installed binary $p (verified as accelrun)" rm -f -- "$p"
      done
    fi
    # dev-host linker workaround — referenced by .cargo/config.toml; removing
    # it breaks local `cargo build --release` until glibc dev files are
    # re-extracted there (documented in docs/decisions.md)
    if [ -d /tmp/fakelib ]; then
      try_item "remove /tmp/fakelib — dev-host linker workaround; see .cargo/config.toml" \
        rm -rf -- /tmp/fakelib
    else
      echo "  /tmp/fakelib — ok"
    fi
    # release-verification clone (docs/releasing.md)
    if [ -d /tmp/ar-test ]; then
      try_item "remove /tmp/ar-test (release-verification clone)" rm -rf -- /tmp/ar-test
    else
      echo "  /tmp/ar-test — ok"
    fi
  fi
fi

# ----------------------------------------------------------------- summary ---

echo
if $DRY_RUN; then
  echo "dry-run complete — nothing was changed."
else
  echo "cleanup finished ($removed artifact(s) removed)."
fi
exit 0
