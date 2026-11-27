#!/usr/bin/env bash
# build.sh — build accelrun with Docker, no Rust toolchain on the host.
#
# "install-nothing" build: the only requirement is a running Docker daemon.
# The multi-stage Dockerfile does the Rust compile; this script just drives
# it and extracts the finished binary to the host, then verifies the result
# (ELF magic + ACCEL_* signature + a live `probe` smoke test) before
# declaring success.
#
# Usage:
#   ./build.sh                      build image + extract ./accelrun
#   ./build.sh --release v0.1.1     also emit accelrun-v0.1.1-<arch> + SHA256SUMS
#   ./build.sh --no-cache           force a clean rebuild (drop docker layers)
#   ./build.sh --keep-image         keep the accelrun image (default: kept)
#   ./build.sh --tag IMG            use a custom image tag (default accelrun:latest)
#   ./build.sh --dry-run            print the commands without running them
#
# The image is a BUILD VEHICLE ONLY: it exists to compile the binary and
# carry it to the host. accelrun runs on the host (it probes the host's
# accelerators and shells out to the host `docker` CLI), never inside the
# image. The image is kept by default only so clean.sh can match/remove it
# like the quickstart's helper container; the throwaway extraction container
# is always removed here.

usage() {
  # print the header comment block (lines 2..first blank line), strip '# '.
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

set -u
shopt -s nullglob

TAG="accelrun:latest"
RELEASE=""
NO_CACHE=false
KEEP_IMAGE=true
DRY_RUN=false

while [ $# -gt 0 ]; do
  case "$1" in
    --release)       RELEASE="$2"; shift 2 ;;
    --release=*)     RELEASE="${1#*=}"; shift ;;
    --tag)           TAG="$2"; shift 2 ;;
    --tag=*)         TAG="${1#*=}"; shift ;;
    --no-cache)      NO_CACHE=true; shift ;;
    --keep-image)    KEEP_IMAGE=true; shift ;;
    --rm-image)      KEEP_IMAGE=false; shift ;;
    --dry-run)       DRY_RUN=true; shift ;;
    -h|--help)       usage; exit 0 ;;
    *) echo "unknown arg: $1 (try --help)" >&2; exit 2 ;;
  esac
done

repo_dir="$(cd "$(dirname "$0")" && pwd)"
cd "$repo_dir" || exit 1

# arch for release artifact naming (consistent with docs/releasing.md)
case "$(uname -m)" in
  x86_64|amd64)  arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *)             arch="$(uname -m)" ;;
esac

# unique throwaway container name — never collides with a real accelrun name
extract_ctr="ar-build-$$"

run() { if $DRY_RUN; then echo "  \$ $*"; else "$@"; fi; }

echo "accelrun build — via Docker (no Rust toolchain on the host)"
echo "  image:   $TAG"
echo "  binary:  $repo_dir/accelrun"
echo "  arch:    $arch"
$DRY_RUN && echo "  (dry-run: printing commands only)"
[ -n "$RELEASE" ] && echo "  release: accelrun-${RELEASE}-${arch} + SHA256SUMS"
echo

# ------------------------------------------------------------------ build ---

if ! docker version >/dev/null 2>&1 && ! $DRY_RUN; then
  echo "docker is not available/running — cannot build." >&2
  echo "install-nothing needs exactly one thing: a working Docker daemon." >&2
  exit 1
fi

echo "── build image ──"
build_args=(docker build -t "$TAG")
$NO_CACHE && build_args+=(--no-cache)
build_args+=(".")
run "${build_args[@]}" || { echo "build failed" >&2; exit 1; }
echo

# ---------------------------------------------------------------- extract ---

echo "── extract binary ──"
run docker create --name "$extract_ctr" "$TAG" \
  || { echo "failed to create extraction container" >&2; exit 1; }
run docker cp "$extract_ctr:/accelrun" "$repo_dir/accelrun" \
  || { echo "failed to copy binary" >&2; exit 1; }
if $DRY_RUN; then
  run docker rm "$extract_ctr"
else
  docker rm "$extract_ctr" >/dev/null
fi
run chmod +x "$repo_dir/accelrun"
echo

if ! $KEEP_IMAGE; then
  echo "── remove image (--rm-image) ──"
  if $DRY_RUN; then
    echo "  \$ docker rmi $TAG"
  else
    docker rmi "$TAG" >/dev/null 2>&1 || true
  fi
  echo
fi

# ----------------------------------------------------------------- verify ---

echo "── verify ──"
if $DRY_RUN; then
  echo "  (skipped in dry-run)"
else
  # evidence, not assumptions (same signatures clean.sh uses): ELF magic +
  # the ACCEL_* contract string baked into the binary.
  head -c 4 "$repo_dir/accelrun" 2>/dev/null | grep -aq ELF \
    || { echo "FAIL: not an ELF binary" >&2; exit 1; }
  grep -aq 'ACCEL_' "$repo_dir/accelrun" 2>/dev/null \
    || { echo "FAIL: missing ACCEL_* signature" >&2; exit 1; }
  echo "  ELF + ACCEL_* signature: ok"
  # live smoke test — probe always succeeds, GPU or not (CPU fallback)
  if "$repo_dir/accelrun" probe >/dev/null 2>&1; then
    echo "  probe smoke test: ok"
  else
    echo "  probe smoke test: FAILED" >&2
    echo "  (binary extracted but does not run on this host)" >&2
    exit 1
  fi
  echo
fi

# ---------------------------------------------------------------- release ---

if [ -n "$RELEASE" ]; then
  echo "── release artifacts ──"
  rel_bin="accelrun-${RELEASE}-${arch}"
  if $DRY_RUN; then
    echo "  \$ cp accelrun $rel_bin"
    echo "  \$ sha256sum $rel_bin >> SHA256SUMS"
  else
    cp "$repo_dir/accelrun" "$repo_dir/$rel_bin"
    # SHA256SUMS is a committed, cumulative record (one line per released
    # version). Upsert this version: drop any prior line for it (so a rebuild
    # can't leave a duplicate or a stale hash), then append the fresh hash.
    if [ -f "$repo_dir/SHA256SUMS" ]; then
      grep -vF "  $rel_bin" "$repo_dir/SHA256SUMS" > "$repo_dir/SHA256SUMS.tmp" \
        && mv "$repo_dir/SHA256SUMS.tmp" "$repo_dir/SHA256SUMS"
    fi
    sha256sum "$rel_bin" >> "$repo_dir/SHA256SUMS"
    echo "  wrote $rel_bin"
    echo "  checksum appended to SHA256SUMS (commit it with the release)"
  fi
  echo
fi

# ----------------------------------------------------------------- summary ---

if $DRY_RUN; then
  echo "dry-run complete — nothing was built."
else
  echo "done: $repo_dir/accelrun"
fi
exit 0
