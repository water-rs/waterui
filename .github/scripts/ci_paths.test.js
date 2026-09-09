import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const workflow = Bun.YAML.parse(
  readFileSync(new URL("../workflows/ci.yml", import.meta.url), "utf8"),
);
const changes = workflow.jobs.changes;
const filters = new Map(
  changes.steps
    .filter((step) => step.uses?.startsWith("dorny/paths-filter@"))
    .map((step) => [step.id, step.with]),
);

function matches(step, name, paths) {
  const patterns = Bun.YAML.parse(step.filters)[name];
  const predicates = patterns.map((pattern) => {
    const negative = pattern.startsWith("!");
    const glob = new Bun.Glob(negative ? pattern.slice(1) : pattern);
    return (path) => glob.match(path) !== negative;
  });
  return paths.some((path) =>
    step["predicate-quantifier"] === "every"
      ? predicates.every((predicate) => predicate(path))
      : predicates.some((predicate) => predicate(path)),
  );
}

function runsCode(paths) {
  return (
    matches(filters.get("prose"), "code", paths) ||
    matches(filters.get("filter"), "workflows", paths)
  );
}

describe("CI path selection", () => {
  test("the job output consumes the positive code result", () => {
    expect(changes.outputs.code).toBe(
      "${{ steps.prose.outputs.code == 'true' || steps.filter.outputs.workflows == 'true' }}",
    );
    expect(workflow.jobs.test.if).toBe("needs.changes.outputs.code == 'true'");
  });

  test.each([
    ["AGENTS.md"],
    ["components/platform/webview/README.md"],
    ["docs/guide.html"],
    ["LICENSE-APACHE"],
    [".github/ISSUE_TEMPLATE/bug.yml"],
  ])("prose does not compile Rust: %s", (path) => {
    expect(runsCode([path])).toBe(false);
  });

  test("mixed prose categories do not compile Rust", () => {
    expect(runsCode(["AGENTS.md", "docs/guide.html", "LICENSE-MIT"])).toBe(false);
  });

  test.each([
    ["src/lib.rs"],
    ["Cargo.toml"],
    ["Cargo.lock"],
    [".gitmodules"],
    ["backends/apple"],
    [".github/workflows/ci.yml"],
    [".github/actions/setup-linux-deps/action.yml"],
    [".github/scripts/ci_paths.test.js"],
  ])("code and build metadata compile Rust: %s", (path) => {
    expect(runsCode([path])).toBe(true);
    expect(runsCode(["AGENTS.md", path])).toBe(true);
  });

  test("workflow and action prose still selects the workflow gate", () => {
    expect(runsCode([".github/workflows/README.md"])).toBe(true);
    expect(runsCode([".github/actions/README.md"])).toBe(true);
  });

  test("an empty diff does not compile Rust", () => {
    expect(runsCode([])).toBe(false);
  });
});
