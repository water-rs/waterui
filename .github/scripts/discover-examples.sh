#!/usr/bin/env bash
set -euo pipefail

waterui_dir="${1:-${WATERUI_DIR:-${GITHUB_WORKSPACE:-$(pwd)}}}"
examples_roots=("${waterui_dir}/examples" "${waterui_dir}/backends/apple/Examples")

examples=()
for examples_root in "${examples_roots[@]}"; do
  [[ -d "${examples_root}" ]] || continue
  while IFS= read -r example; do
    examples+=("${example}")
  done < <(
    find "${examples_root}" -mindepth 1 -maxdepth 1 -type d -print0 |
      while IFS= read -r -d '' example_dir; do
        if [[ -f "${example_dir}/Cargo.toml" ]]; then
          basename "${example_dir}"
        fi
      done
  )
done
# Sort + unique: a name present in both roots is one example entry (the
# consumers resolve root examples/ first, then backends/apple/Examples/).
IFS=$'\n' read -r -d '' -a examples < <(printf '%s\n' "${examples[@]}" | sort -u; printf '\0')
unset IFS

if (( ${#examples[@]} == 0 )); then
  echo "::error::No runnable examples found under ${examples_roots[*]}"
  exit 1
fi

printf '%s\n' "${examples[@]}"
