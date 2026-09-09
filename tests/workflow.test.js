const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");

const WORKFLOW = ".github/workflows/ci.yml";

/**
 * Reads the folded `if:` expression of a job, without a YAML dependency.
 */
function jobCondition(job) {
  const lines = fs.readFileSync(WORKFLOW, "utf8").split("\n");
  const start = lines.indexOf(`  ${job}:`);
  assert.notStrictEqual(start, -1, `job ${job} not found in ${WORKFLOW}`);

  const body = [];
  let collecting = false;
  for (const line of lines.slice(start + 1)) {
    const indent = line.search(/\S/);
    if (indent !== -1 && indent <= 2) break; // next job
    if (collecting) {
      if (indent !== -1 && indent <= 4) break; // next key of this job
      body.push(line.trim());
      continue;
    }
    if (line.trim() === "if: >-") collecting = true;
  }
  assert.ok(body.length > 0, `job ${job} has no folded if: expression`);
  return body.join(" ");
}

/**
 * Reads the top-level `on:` trigger names, without a YAML dependency.
 */
function declaredTriggers() {
  const lines = fs.readFileSync(WORKFLOW, "utf8").split("\n");
  const start = lines.indexOf("on:");
  assert.notStrictEqual(start, -1, `no on: block in ${WORKFLOW}`);

  const triggers = [];
  for (const line of lines.slice(start + 1)) {
    const indent = line.search(/\S/);
    if (indent === -1) continue;
    if (indent === 0) break; // next top-level key
    if (indent === 2) {
      const name = line.trim().replace(/:.*$/, "");
      if (name) triggers.push(name);
    }
  }
  return triggers;
}

const CONTEXT_KEYS = [
  "github.event.pull_request.user.login",
  "github.event_name",
  "github.head_ref",
];

/**
 * Evaluates the subset of GitHub expression syntax the workflow uses. Anything
 * outside that subset throws, so an unsupported operator fails the test rather
 * than quietly evaluating to the wrong branch.
 */
function evaluate(expression, context) {
  let remaining = expression;
  for (const key of CONTEXT_KEYS) {
    remaining = remaining.split(key).join(JSON.stringify(context[key] ?? ""));
  }
  assert.ok(
    !remaining.includes("github."),
    `unhandled context reference in: ${remaining}`
  );

  const js = remaining.replace(
    /startsWith\(\s*("(?:[^"\\]|\\.)*")\s*,\s*'([^']*)'\s*\)/g,
    (_match, subject, prefix) => `${subject}.startsWith(${JSON.stringify(prefix)})`
  );
  const supported =
    /^(?:\s|\(|\)|&&|\|\||==|!=|"(?:[^"\\]|\\.)*"|'[^']*'|\.startsWith|true|false)+$/;
  assert.ok(supported.test(js), `unsupported expression syntax: ${js}`);

  return Function(`"use strict"; return (${js});`)();
}

const RELEASE_BOT = "github-actions[bot]";
const RELEASE_BRANCH = "release-please--branches--main--components--pidjezdy";

const CASES = [
  {
    name: "the Release Please pull request runs the matrix",
    context: {
      "github.event_name": "pull_request",
      "github.event.pull_request.user.login": RELEASE_BOT,
      "github.head_ref": RELEASE_BRANCH,
    },
    expected: true,
  },
  {
    name: "a fork pull request copying the release branch name does not",
    context: {
      "github.event_name": "pull_request",
      "github.event.pull_request.user.login": "outside-contributor",
      "github.head_ref": RELEASE_BRANCH,
    },
    expected: false,
  },
  {
    name: "the release bot on a non-release branch does not",
    context: {
      "github.event_name": "pull_request",
      "github.event.pull_request.user.login": RELEASE_BOT,
      "github.head_ref": "dependabot/cargo/serde-1.0.230",
    },
    expected: false,
  },
  {
    name: "an ordinary pull request does not",
    context: {
      "github.event_name": "pull_request",
      "github.event.pull_request.user.login": "Malanius",
      "github.head_ref": "feature/some-change",
    },
    expected: false,
  },
  {
    name: "a push to main does not",
    context: { "github.event_name": "push" },
    expected: false,
  },
  {
    name: "a manual dispatch does",
    context: { "github.event_name": "workflow_dispatch" },
    expected: true,
  },
];

test("the portability matrix runs only before a release or on demand", () => {
  const condition = jobCondition("rust-portability");
  for (const { name, context, expected } of CASES) {
    assert.strictEqual(evaluate(condition, context), expected, name);
  }
});

test("the workflow still declares the manual dispatch trigger", () => {
  // The condition can name workflow_dispatch while the trigger is gone, which
  // would silently make an on-demand matrix run impossible.
  assert.ok(
    declaredTriggers().includes("workflow_dispatch"),
    `on: declares ${JSON.stringify(declaredTriggers())}`
  );
});

test("the evaluator rejects syntax it does not model", () => {
  assert.throws(
    () => evaluate("contains(github.head_ref, 'release')", {
      "github.head_ref": "release-please--x",
    }),
    /unsupported expression syntax/
  );
});
