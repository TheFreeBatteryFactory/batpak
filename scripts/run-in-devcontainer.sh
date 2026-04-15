#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runtime="${OCI_RUNTIME:-docker}"
image_tag="${BATPAK_DEVCONTAINER_IMAGE:-batpak-devcontainer}"
skip_build="${BATPAK_DEVCONTAINER_SKIP_BUILD:-0}"
dockerfile="${repo_root}/.devcontainer/Dockerfile"
image_hash_label="io.batpak.devcontainer-hash"

if [[ $# -eq 0 ]]; then
  echo "scripts/run-in-devcontainer.sh requires an explicit command." >&2
  echo "Example: ./scripts/run-in-devcontainer.sh cargo xtask ci" >&2
  exit 1
fi

hash_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  else
    echo "Need sha256sum or shasum to fingerprint ${path}" >&2
    exit 1
  fi
}

current_dockerfile_hash="$(hash_file "${dockerfile}")"
existing_image_hash="$("${runtime}" image inspect "${image_tag}" \
  --format "{{ index .Config.Labels \"${image_hash_label}\" }}" 2>/dev/null || true)"

if [[ "${skip_build}" != "1" ]]; then
  if [[ -n "${existing_image_hash}" && "${existing_image_hash}" == "${current_dockerfile_hash}" ]]; then
    echo "Reusing local devcontainer image '${image_tag}' (Dockerfile unchanged)."
  else
    "${runtime}" build \
      --label "${image_hash_label}=${current_dockerfile_hash}" \
      -f "${dockerfile}" \
      -t "${image_tag}" \
      "${repo_root}"
  fi
elif ! "${runtime}" image inspect "${image_tag}" >/dev/null 2>&1; then
  echo "BATPAK_DEVCONTAINER_SKIP_BUILD=1 was set but image '${image_tag}' is not available locally." >&2
  exit 1
fi

"${runtime}" run --rm \
  -e DEVCONTAINER=1 \
  -e CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}" \
  -e PROPTEST_CASES="${PROPTEST_CASES:-256}" \
  -v "${repo_root}:/workspace/batpak" \
  -w /workspace/batpak \
  "${image_tag}" \
  "$@"
