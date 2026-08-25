#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
ARTIFACTS_ROOT="${REPO_ROOT}/artifacts/linux-cli"

platforms=("x64")
format="all"
version=""

usage() {
  cat <<'EOF'
Usage: scripts/build-linux-cli.sh [options]

Options:
  --platforms x86,x64   Target platforms to build. Default: x64
  --format bin|deb|all  Output format. Default: all
  --version VERSION     Override package version. Default: Cargo.toml version
  -h, --help            Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --platforms)
      IFS=',' read -r -a platforms <<< "${2:-}"
      shift 2
      ;;
    --format)
      format="${2:-}"
      shift 2
      ;;
    --version)
      version="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required but was not found in PATH." >&2
  exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required but was not found in PATH." >&2
  exit 1
fi

if [[ -z "${version}" ]]; then
  version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${REPO_ROOT}/Cargo.toml" | head -n 1)"
fi

if [[ -z "${version}" ]]; then
  echo "Unable to resolve version from Cargo.toml." >&2
  exit 1
fi

case "${format}" in
  bin|deb|all)
    ;;
  *)
    echo "Unsupported format: ${format}. Use bin, deb, or all." >&2
    exit 1
    ;;
esac

resolve_platform() {
  case "$1" in
    x86)
      echo "i686-unknown-linux-gnu:i386:linux-x86"
      ;;
    x64)
      echo "x86_64-unknown-linux-gnu:amd64:linux-x64"
      ;;
    *)
      echo "Unsupported platform '$1'. Use x86 or x64." >&2
      exit 1
      ;;
  esac
}

ensure_target() {
  local target="$1"
  if ! rustup target list --installed | grep -qx "${target}"; then
    rustup target add "${target}"
  fi
}

build_binary() {
  local target="$1"
  if command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1; then
    cargo zigbuild --release --bin vaultpilot-cli --target "${target}"
    return
  fi

  cargo build --release --bin vaultpilot-cli --target "${target}"
}

package_deb() {
  local target="$1"
  local deb_arch="$2"
  local channel="$3"
  local binary_path="${REPO_ROOT}/target/${target}/release/vaultpilot-cli"
  local output_dir="${ARTIFACTS_ROOT}/packages/${channel}"
  local staging_root
  staging_root="$(mktemp -d)"
  # Clean up temp directory on function return (success or failure) — issue #726
  trap 'rm -rf "${staging_root}"' RETURN
  local staging_dir="${staging_root}/vaultpilot-cli"
  local package_name="vaultpilot-cli_${version}_${deb_arch}.deb"

  mkdir -p \
    "${staging_dir}/DEBIAN" \
    "${staging_dir}/usr/bin" \
    "${staging_dir}/usr/share/doc/vaultpilot-cli"
  chmod 0755 "${staging_dir}" "${staging_dir}/DEBIAN" "${staging_dir}/usr" "${staging_dir}/usr/bin" "${staging_dir}/usr/share" "${staging_dir}/usr/share/doc" "${staging_dir}/usr/share/doc/vaultpilot-cli"

  install -m 0755 "${binary_path}" "${staging_dir}/usr/bin/vaultpilot-cli"

  if [[ -f "${REPO_ROOT}/README.md" ]]; then
    install -m 0644 "${REPO_ROOT}/README.md" "${staging_dir}/usr/share/doc/vaultpilot-cli/README.md"
  fi

  if [[ -f "${REPO_ROOT}/LICENSE" ]]; then
    install -m 0644 "${REPO_ROOT}/LICENSE" "${staging_dir}/usr/share/doc/vaultpilot-cli/LICENSE"
  fi

  cat > "${staging_dir}/DEBIAN/control" <<EOF
Package: vaultpilot-cli
Version: ${version}
Section: utils
Priority: optional
Architecture: ${deb_arch}
Maintainer: jy
Description: VaultPilot CLI for local knowledge-base search and grounded AI chat
 This package contains the headless VaultPilot CLI for Linux.
EOF
  chmod 0644 "${staging_dir}/DEBIAN/control"

  mkdir -p "${output_dir}"
  dpkg-deb --build "${staging_dir}" "${output_dir}/${package_name}"
}

mkdir -p "${ARTIFACTS_ROOT}/bin" "${ARTIFACTS_ROOT}/packages"
rm -rf "${ARTIFACTS_ROOT}/staging"
built_channels=()

for platform in "${platforms[@]}"; do
  IFS=':' read -r rust_target deb_arch channel <<< "$(resolve_platform "${platform}")"
  built_channels+=("${channel}")

  echo "Building vaultpilot-cli for ${platform} (${rust_target})..."
  ensure_target "${rust_target}"
  rm -rf "${ARTIFACTS_ROOT}/bin/${channel}" "${ARTIFACTS_ROOT}/packages/${channel}"
  build_binary "${rust_target}"

  binary_source="${REPO_ROOT}/target/${rust_target}/release/vaultpilot-cli"
  binary_output_dir="${ARTIFACTS_ROOT}/bin/${channel}"
  mkdir -p "${binary_output_dir}"
  install -m 0755 "${binary_source}" "${binary_output_dir}/vaultpilot-cli"

  if [[ "${format}" == "deb" || "${format}" == "all" ]]; then
    if ! command -v dpkg-deb >/dev/null 2>&1; then
      echo "dpkg-deb is required to build .deb packages." >&2
      exit 1
    fi
    package_deb "${rust_target}" "${deb_arch}" "${channel}"
  fi
done

echo
echo "Build complete. Artifacts:"
for channel in "${built_channels[@]}"; do
  find "${ARTIFACTS_ROOT}/bin/${channel}" "${ARTIFACTS_ROOT}/packages/${channel}" -type f 2>/dev/null
done | sort
