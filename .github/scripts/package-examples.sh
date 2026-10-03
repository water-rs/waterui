#!/usr/bin/env bash
# Nightly build job. Packages every runnable example once for one platform
# (`water package --release`) and stages the result as the packaged-<platform>
# directory the capture shards download (#218 — the shards used to each pay
# the same cold release build):
#
#   apps/<example>/<App>.app        the packaged bundle
#   apps/<example>/libwaterui_app.a the Rust archive `water package` leaves
#                                   beside the bundle
#   logs/<platform>-<example>.log   the example's package log, named exactly
#                                   as the shard's run log is — the shard
#                                   copies it into its logs dir so the
#                                   uploaded artifacts look unchanged
#   package-sizes-<platform>.json   the per-example size entries, recorded
#                                   here because they are a property of the
#                                   package, not of the capture
#
# An example `water package` refuses as platform-impossible ("unsupported
# for" in the log — e.g. a CEF WebView on iOS, #154) is recorded as null and
# skipped, matching the shard's historical treatment. Any other packaging
# failure is collected with its log tail and fails the job after every
# example has been attempted.
#
# EXAMPLES (comma-separated) restricts the set to an explicit list — the
# build matrix legs each pass their assigned group this way, the same
# convention run-e2e-shard.sh honours (#252). Unset means every discovered
# example.
set -euo pipefail

platform="${PLATFORM:?PLATFORM must be ios or macos}"
workspace="${GITHUB_WORKSPACE:-$(pwd)}"
waterui_dir="${WATERUI_DIR:?WATERUI_DIR must point at the prepared waterui checkout}"
logs_dir="${EXAMPLE_LOG_DIR:-${workspace}/e2e-logs}"
out_dir="${PACKAGE_OUT_DIR:-${workspace}/packaged-${platform}}"

case "${platform}" in
  ios) package_platform="ios-simulator" ;;
  macos) package_platform="macos" ;;
  *)
    echo "::error::Unsupported platform '${platform}'. Expected ios or macos."
    exit 1
    ;;
esac

mkdir -p "${logs_dir}" "${out_dir}/apps" "${out_dir}/logs"

# A failed discovery exits nonzero here — its ::error line was captured into
# `discovered`, so reprint it before leaving. Reading through a substitution
# instead of process substitution keeps the exit status; a `::error` line
# captured as an example name would otherwise be packaged as one.
if ! discovered="$("${workspace}/.github/scripts/discover-examples.sh" "${waterui_dir}")"; then
  printf '%s\n' "${discovered}"
  exit 1
fi

all_examples=()
while IFS= read -r example; do
  [[ -n "${example}" ]] || continue
  all_examples+=("${example}")
done <<< "${discovered}"

declare -a selected_examples=()
if [[ -n "${EXAMPLES:-}" ]]; then
  # An explicit list selects exactly those examples; a name discovery did
  # not find is a typo, not an empty group, so it fails the job fast. Same
  # contract the shard script applies to its EXAMPLES input.
  IFS=',' read -ra requested_examples <<< "${EXAMPLES}"
  for example in ${requested_examples[@]+"${requested_examples[@]}"}; do
    example="$(printf '%s' "${example}" | tr -d '[:space:]')"
    [[ -n "${example}" ]] || continue
    known=0
    for candidate in ${all_examples[@]+"${all_examples[@]}"}; do
      if [[ "${candidate}" == "${example}" ]]; then
        known=1
        break
      fi
    done
    if (( known == 0 )); then
      echo "::error::Requested example '${example}' is not a runnable example under ${waterui_dir}/examples."
      exit 1
    fi
    selected_examples+=("${example}")
  done
else
  for example in ${all_examples[@]+"${all_examples[@]}"}; do
    selected_examples+=("${example}")
  done
fi

if (( ${#selected_examples[@]} == 0 )); then
  echo "::error::No examples to package for ${platform}."
  exit 1
fi

echo "Packaging ${#selected_examples[@]} examples for ${platform}: ${selected_examples[*]}"

size_entries="${out_dir}/.size-${platform}.entries"
: > "${size_entries}"

# `water package` emits `Packaged at <path>` on success; resolve the bundle
# it names. Same contract the shard script uses on its inline builds.
find_packaged_app() {
  local log_file="$1" app_path
  app_path="$(sed -n 's/.*Packaged at //p' "${log_file}" | tail -1 | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r')"
  [[ -n "${app_path}" && -d "${app_path}" ]] && echo "${app_path}"
}

declare -a failures=()

for example in ${selected_examples[@]+"${selected_examples[@]}"}; do
  example_path="${waterui_dir}/examples/${example}"
  [[ -d "${example_path}" ]] || example_path="${waterui_dir}/backends/apple/Examples/${example}"
  run_log="${logs_dir}/${platform}-${example}.log"

  echo "::group::package ${example} (${platform})"

  app_path=""
  if water package --platform "${package_platform}" --backend apple --release \
      --path "${example_path}" > "${run_log}" 2>&1; then
    app_path="$(find_packaged_app "${run_log}")"
  fi

  # The package log crosses to the shard inside the artifact regardless of
  # the outcome — the shard reads "unsupported for" from it to reproduce the
  # skip an inline packaging failure would have produced.
  cp "${run_log}" "${out_dir}/logs/${platform}-${example}.log"

  if [[ -n "${app_path}" ]]; then
    dest="${out_dir}/apps/${example}"
    mkdir -p "${dest}"
    # ditto preserves the bundle's symlinks, permissions and ad-hoc code
    # signature on the way into the staging dir; the tar the workflow builds
    # from it preserves them across the artifact round trip.
    ditto "${app_path}" "${dest}/$(basename "${app_path}")"
    archive="$(dirname "${app_path}")/libwaterui_app.a"
    [[ -f "${archive}" ]] && cp "${archive}" "${dest}/"

    app_bytes="$(find "${app_path}" -type f -exec stat -f%z {} + | awk '{s+=$1} END {print s}')"
    if [[ "${platform}" == "macos" ]]; then
      executable="${app_path}/Contents/MacOS/$(basename "${app_path}" .app)"
    else
      executable="${app_path}/$(basename "${app_path}" .app)"
    fi
    executable_bytes="$(stat -f%z "${executable}")"
    printf '  "%s": { "app_bytes": %s, "executable_bytes": %s },\n' \
      "${example}" "${app_bytes}" "${executable_bytes}" >> "${size_entries}"
    echo "::notice::${example} packaged: .app=${app_bytes}B executable=${executable_bytes}B (${platform})"
  elif grep -qi "unsupported for" "${run_log}"; then
    echo "::notice::${example} is unsupported on ${platform}; skipping."
    printf '  "%s": null,\n' "${example}" >> "${size_entries}"
  else
    echo "::error::water package failed for ${example} (${platform}). Log tail:"
    tail -n 60 "${run_log}" || true
    failures+=("${example}")
    printf '  "%s": null,\n' "${example}" >> "${size_entries}"
  fi

  echo "::endgroup::"
done

{
  echo "{"
  sed '$ s/,$//' "${size_entries}"
  echo "}"
} > "${out_dir}/package-sizes-${platform}.json"
rm -f "${size_entries}"

if (( ${#failures[@]} > 0 )); then
  echo "::error::${#failures[@]} example(s) failed to package on ${platform}: ${failures[*]}"
  exit 1
fi
